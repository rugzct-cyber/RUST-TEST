"use client";

import { useState, useRef, useEffect, useMemo, ClipboardEvent } from "react";
import type { Position } from "@/lib/types";

// Sanitize a pasted/typed price string:
// "69,396.15" → "69396.15"  (comma = thousands separator, dot = decimal)
// "69.396,15" → "69396.15"  (dot = thousands separator, comma = decimal — EU format)
// "69396.15"  → "69396.15"  (already clean)
function sanitizePrice(raw: string): string {
    const trimmed = raw.trim();
    const lastComma = trimmed.lastIndexOf(",");
    const lastDot = trimmed.lastIndexOf(".");

    if (lastComma > -1 && lastDot > -1) {
        if (lastDot > lastComma) {
            // Format: 69,396.15 → comma is thousands sep
            return trimmed.replace(/,/g, "");
        } else {
            // Format: 69.396,15 → dot is thousands sep, comma is decimal
            return trimmed.replace(/\./g, "").replace(",", ".");
        }
    }
    if (lastComma > -1 && lastDot === -1) {
        // Only commas: could be "69396,15" (EU decimal) or "69,396" (thousands)
        // If digits after last comma are exactly 3 and there are digits before → thousands sep
        const afterComma = trimmed.slice(lastComma + 1);
        if (afterComma.length === 3 && /^\d+$/.test(afterComma)) {
            // Ambiguous, but treat as thousands separator (e.g. "69,396")
            return trimmed.replace(/,/g, "");
        }
        // Otherwise treat comma as decimal (e.g. "69396,15")
        return trimmed.replace(",", ".");
    }
    // Only dots or no separator — already fine
    return trimmed;
}

function handlePriceChange(value: string, setter: (v: string) => void) {
    // Allow digits, dots, commas during typing
    const cleaned = value.replace(/[^0-9.,]/g, "");
    setter(cleaned);
}

function handlePricePaste(e: ClipboardEvent<HTMLInputElement>, setter: (v: string) => void) {
    e.preventDefault();
    const pasted = e.clipboardData.getData("text");
    setter(sanitizePrice(pasted));
}

interface AddPositionFormProps {
    availableTokens: string[];
    availableExchanges: string[];
    onAdd: (position: Position) => void;
}

export function AddPositionForm({ availableTokens, availableExchanges, onAdd }: AddPositionFormProps) {
    const [token, setToken] = useState("");
    const [longExchange, setLongExchange] = useState("");
    const [shortExchange, setShortExchange] = useState("");
    const [entryPriceLong, setEntryPriceLong] = useState("");
    const [entryPriceShort, setEntryPriceShort] = useState("");
    const [tokenAmount, setTokenAmount] = useState("");
    const [formError, setFormError] = useState<string | null>(null);
    const [showSuggestions, setShowSuggestions] = useState(false);
    const tokenInputRef = useRef<HTMLDivElement>(null);

    const filteredTokens = useMemo(() => {
        if (!token) return availableTokens;
        const search = token.toUpperCase();
        return availableTokens.filter((t) => t.toUpperCase().includes(search));
    }, [token, availableTokens]);

    useEffect(() => {
        const handleClickOutside = (event: MouseEvent) => {
            if (tokenInputRef.current && !tokenInputRef.current.contains(event.target as Node)) {
                setShowSuggestions(false);
            }
        };
        document.addEventListener("mousedown", handleClickOutside);
        return () => document.removeEventListener("mousedown", handleClickOutside);
    }, []);

    const handleAdd = () => {
        if (!token || !longExchange || !shortExchange || !entryPriceLong || !entryPriceShort || !tokenAmount) {
            setFormError("Remplis tous les champs");
            return;
        }
        if (longExchange === shortExchange) {
            setFormError("Les exchanges doivent être différents");
            return;
        }
        setFormError(null);

        // Store the token exactly as it appears in the WS data (e.g. "BTC", "ETH", "SOL")
        const normalizedToken = token.toUpperCase().replace(/-USD$/, "").replace(/-PERP$/, "");
        const pLong = parseFloat(sanitizePrice(entryPriceLong));
        const pShort = parseFloat(sanitizePrice(entryPriceShort));

        const newPosition: Position = {
            id: Date.now().toString(),
            token: normalizedToken,
            longExchange,
            shortExchange,
            entryPriceLong: pLong,
            entryPriceShort: pShort,
            tokenAmount: parseFloat(tokenAmount),
            entrySpread: pShort > 0 ? ((pShort - pLong) / pLong) * 100 : 0,
            timestamp: Date.now(),
        };

        onAdd(newPosition);
        setToken("");
        setLongExchange("");
        setShortExchange("");
        setEntryPriceLong("");
        setEntryPriceShort("");
        setTokenAmount("");
    };

    const inputClass =
        "w-full px-3 py-2 bg-secondary border border-border rounded-md text-foreground text-sm focus:outline-none focus:border-ring";
    const labelClass = "block text-xs text-muted-foreground mb-1";

    return (
        <div className="bg-card border border-border rounded-lg p-4">
            <h2 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-4">
                Nouvelle Position
            </h2>

            {/* Token with autocomplete */}
            <div className="mb-3 relative" ref={tokenInputRef}>
                <label className={labelClass}>Token</label>
                <input
                    type="text"
                    placeholder="BTC, ETH, SOL..."
                    value={token}
                    onChange={(e) => { setToken(e.target.value); setShowSuggestions(true); setFormError(null); }}
                    onFocus={() => setShowSuggestions(true)}
                    className={inputClass}
                />
                {showSuggestions && filteredTokens.length > 0 && (
                    <div className="absolute top-full left-0 right-0 mt-1 bg-card border border-border rounded-md max-h-[200px] overflow-y-auto z-50 shadow-lg">
                        {filteredTokens.map((t) => (
                            <div
                                key={t}
                                className="px-3 py-2 cursor-pointer text-sm text-foreground hover:bg-primary/10 transition-colors"
                                onClick={() => { setToken(t); setShowSuggestions(false); }}
                            >
                                {t}
                            </div>
                        ))}
                    </div>
                )}
            </div>

            {/* Exchange LONG */}
            <div className="mb-3">
                <label className={labelClass}>Exchange LONG</label>
                <select value={longExchange} onChange={(e) => { setLongExchange(e.target.value); setFormError(null); }} className={inputClass}>
                    <option value="">Sélectionner...</option>
                    {availableExchanges.map((ex) => (
                        <option key={ex} value={ex}>{ex.toUpperCase()}</option>
                    ))}
                </select>
            </div>

            {/* Exchange SHORT */}
            <div className="mb-3">
                <label className={labelClass}>Exchange SHORT</label>
                <select value={shortExchange} onChange={(e) => { setShortExchange(e.target.value); setFormError(null); }} className={inputClass}>
                    <option value="">Sélectionner...</option>
                    {availableExchanges.map((ex) => (
                        <option key={ex} value={ex}>{ex.toUpperCase()}</option>
                    ))}
                </select>
            </div>

            {/* Entry prices */}
            <div className="mb-3">
                <label className={labelClass}>Prix d&apos;entrée LONG ($)</label>
                <input type="text" inputMode="decimal" placeholder="100.00" value={entryPriceLong}
                    onChange={(e) => { handlePriceChange(e.target.value, setEntryPriceLong); setFormError(null); }}
                    onPaste={(e) => { handlePricePaste(e, setEntryPriceLong); setFormError(null); }}
                    className={inputClass} />
            </div>
            <div className="mb-3">
                <label className={labelClass}>Prix d&apos;entrée SHORT ($)</label>
                <input type="text" inputMode="decimal" placeholder="100.00" value={entryPriceShort}
                    onChange={(e) => { handlePriceChange(e.target.value, setEntryPriceShort); setFormError(null); }}
                    onPaste={(e) => { handlePricePaste(e, setEntryPriceShort); setFormError(null); }}
                    className={inputClass} />
            </div>

            {/* Token amount */}
            <div className="mb-3">
                <label className={labelClass}>Nombre de tokens</label>
                <input type="number" step="0.0001" placeholder="1.5" value={tokenAmount}
                    onChange={(e) => { setTokenAmount(e.target.value); setFormError(null); }} className={inputClass} />
            </div>

            {/* Error */}
            {formError && (
                <div className="px-3 py-2 mb-3 bg-destructive/15 border border-destructive/30 rounded-md text-destructive text-sm font-medium">
                    ⚠️ {formError}
                </div>
            )}

            {/* Submit */}
            <button
                onClick={handleAdd}
                className="w-full py-2.5 bg-primary hover:bg-ring text-primary-foreground font-semibold text-sm rounded-md cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]"
            >
                + Ajouter Position
            </button>
        </div>
    );
}
