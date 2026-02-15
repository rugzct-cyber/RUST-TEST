"use client";

import { useState, type ClipboardEvent } from "react";
import type { Position, ExitSpreadData } from "@/lib/types";

// Sanitize a pasted/typed price string:
// "69,396.15" → "69396.15"  (comma = thousands separator, dot = decimal)
// "69.396,15" → "69396.15"  (dot = thousands separator, comma = decimal — EU format)
function sanitizePrice(raw: string): string {
    const trimmed = raw.trim();
    const lastComma = trimmed.lastIndexOf(",");
    const lastDot = trimmed.lastIndexOf(".");
    if (lastComma > -1 && lastDot > -1) {
        if (lastDot > lastComma) return trimmed.replace(/,/g, "");
        return trimmed.replace(/\./g, "").replace(",", ".");
    }
    if (lastComma > -1 && lastDot === -1) {
        const afterComma = trimmed.slice(lastComma + 1);
        if (afterComma.length === 3 && /^\d+$/.test(afterComma)) return trimmed.replace(/,/g, "");
        return trimmed.replace(",", ".");
    }
    return trimmed;
}

interface PositionDetailProps {
    position: Position;
    exitSpreadData: ExitSpreadData | null;
    currentPnL: number | null;
    onUpdatePosition: (updated: Position) => void;
}

export function PositionDetail({ position, exitSpreadData, currentPnL, onUpdatePosition }: PositionDetailProps) {
    const [editingField, setEditingField] = useState<"entryPriceLong" | "entryPriceShort" | "tokenAmount" | null>(null);
    const [editValue, setEditValue] = useState("");

    const handleStartEdit = (field: "entryPriceLong" | "entryPriceShort" | "tokenAmount") => {
        setEditingField(field);
        setEditValue(position[field].toString());
    };

    const handleSaveEdit = () => {
        if (!editingField) return;
        const newValue = parseFloat(sanitizePrice(editValue));
        if (isNaN(newValue) || newValue <= 0) { setEditingField(null); return; }
        onUpdatePosition({ ...position, [editingField]: newValue });
        setEditingField(null);
    };

    const handleEditKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === "Enter") handleSaveEdit();
        else if (e.key === "Escape") { setEditingField(null); setEditValue(""); }
    };

    const entrySpreadDollar = position.entryPriceShort - position.entryPriceLong;
    const entrySpreadPct = ((entrySpreadDollar / position.entryPriceLong) * 100).toFixed(4);

    const statCardBase = "bg-secondary border border-border rounded-lg p-4 text-center";
    const statLabel = "block text-[0.625rem] text-muted-foreground uppercase tracking-wide mb-2";
    const statValue = "text-xl font-bold font-mono";

    const renderEditableField = (
        field: "entryPriceLong" | "entryPriceShort" | "tokenAmount",
        label: string,
        value: number,
        format: (v: number) => string,
        step: string
    ) => (
        <div
            className={`${statCardBase} cursor-pointer hover:border-ring hover:bg-primary/10 transition-all`}
            onClick={() => handleStartEdit(field)}
            title="Cliquer pour modifier"
        >
            <span className={statLabel}>{label}</span>
            {editingField === field ? (
                <input
                    type="text"
                    inputMode="decimal"
                    className="w-full p-2 bg-background border border-primary rounded-md text-foreground text-xl font-bold font-mono text-center focus:outline-none focus:border-ring"
                    value={editValue}
                    onChange={(e) => setEditValue(e.target.value.replace(/[^0-9.,]/g, ""))}
                    onPaste={(e: ClipboardEvent<HTMLInputElement>) => { e.preventDefault(); setEditValue(sanitizePrice(e.clipboardData.getData("text"))); }}
                    onBlur={handleSaveEdit}
                    onKeyDown={handleEditKeyDown}
                    autoFocus
                    onClick={(e) => e.stopPropagation()}
                />
            ) : (
                <span className={statValue}>{format(value)}</span>
            )}
        </div>
    );

    return (
        <>
            {/* Header */}
            <div className="mb-6">
                <h2 className="text-2xl font-bold text-foreground mb-1">
                    {position.token}
                </h2>
                <span className="text-xs text-muted-foreground uppercase tracking-wide">
                    LONG {position.longExchange.toUpperCase()} / SHORT {position.shortExchange.toUpperCase()}
                </span>
            </div>

            {/* Stats Row 1 — Entry */}
            <div className="grid grid-cols-4 gap-3 mb-4">
                {renderEditableField("entryPriceLong", "ENTRÉE LONG", position.entryPriceLong, (v) => `$${v.toFixed(2)}`, "0.01")}
                {renderEditableField("entryPriceShort", "ENTRÉE SHORT", position.entryPriceShort, (v) => `$${v.toFixed(2)}`, "0.01")}
                <div className={statCardBase}>
                    <span className={statLabel}>SPREAD ENTRÉE</span>
                    <span className={`${statValue} ${entrySpreadDollar > 0 ? "text-green-400" : "text-red-400"}`}>
                        ${entrySpreadDollar.toFixed(2)} ({entrySpreadPct}%)
                    </span>
                </div>
                {renderEditableField("tokenAmount", "TOKENS", position.tokenAmount, (v) => `${v}`, "0.0001")}
            </div>

            {/* Stats Row 2 — Live */}
            <div className="grid grid-cols-4 gap-3 mb-4">
                <div className={statCardBase}>
                    <span className={statLabel}>BID ACTUEL (LONG)</span>
                    <span className={statValue}>${exitSpreadData?.longBid.toFixed(2) || "-"}</span>
                </div>
                <div className={statCardBase}>
                    <span className={statLabel}>ASK ACTUEL (SHORT)</span>
                    <span className={statValue}>${exitSpreadData?.shortAsk.toFixed(2) || "-"}</span>
                </div>
                <div className={statCardBase}>
                    <span className={statLabel}>SPREAD SORTIE</span>
                    <span className={`${statValue} ${exitSpreadData?.isInProfit ? "text-green-400" : "text-red-400"}`}>
                        ${exitSpreadData ? (exitSpreadData.longBid - exitSpreadData.shortAsk).toFixed(2) : "-"} ({exitSpreadData?.exitSpread.toFixed(4) || "-"}%)
                    </span>
                </div>
                <div className={statCardBase}>
                    <span className={statLabel}>PnL $ (TOTAL)</span>
                    <span className={`${statValue} ${exitSpreadData?.isInProfit ? "text-green-400" : "text-red-400"}`}>
                        ${currentPnL?.toFixed(2) || "-"}
                    </span>
                </div>
            </div>

            {/* Exchange Price Details */}
            <div className="grid grid-cols-2 gap-4 mb-4">
                <div className="bg-secondary border border-border rounded-lg p-3">
                    <span className="block text-xs text-primary font-semibold mb-2">
                        {position.longExchange.toUpperCase()} (LONG)
                    </span>
                    <div className="flex gap-4 text-sm text-muted-foreground font-mono">
                        <span>Bid: ${exitSpreadData?.longBid.toFixed(4) || "-"}</span>
                        <span>Ask: ${exitSpreadData?.longAsk.toFixed(4) || "-"}</span>
                    </div>
                </div>
                <div className="bg-secondary border border-border rounded-lg p-3">
                    <span className="block text-xs text-primary font-semibold mb-2">
                        {position.shortExchange.toUpperCase()} (SHORT)
                    </span>
                    <div className="flex gap-4 text-sm text-muted-foreground font-mono">
                        <span>Bid: ${exitSpreadData?.shortBid.toFixed(4) || "-"}</span>
                        <span>Ask: ${exitSpreadData?.shortAsk.toFixed(4) || "-"}</span>
                    </div>
                </div>
            </div>

            {/* Exit Spread Chart — TODO */}
            <div className="bg-secondary border border-border rounded-lg p-4">
                <div className="flex items-center justify-between mb-4">
                    <div>
                        <span className="text-sm font-semibold text-foreground">Historique Exit Spread</span>
                        <span className="ml-2 text-xs text-muted-foreground">
                            {position.longExchange.toUpperCase()} BID vs {position.shortExchange.toUpperCase()} ASK
                        </span>
                    </div>
                    <div className="flex gap-1">
                        {(["24H", "7D", "30D", "ALL"] as const).map((range) => (
                            <button
                                key={range}
                                className="px-3 py-1 text-xs rounded-md bg-card border border-border text-muted-foreground hover:border-ring transition-colors cursor-not-allowed opacity-50"
                                disabled
                            >
                                {range}
                            </button>
                        ))}
                    </div>
                </div>
                <div className="flex items-center gap-6 mb-4">
                    <div>
                        <span className="text-[0.625rem] text-muted-foreground uppercase tracking-wide">EXIT SPREAD ACTUEL</span>
                        <span className={`block text-sm font-bold ${(exitSpreadData?.exitSpread ?? 0) > 0 ? "text-green-400" : "text-red-400"}`}>
                            {exitSpreadData?.exitSpread.toFixed(4) || "0.0000"}%
                        </span>
                    </div>
                    <div>
                        <span className="text-[0.625rem] text-muted-foreground uppercase tracking-wide">SPREAD ENTRÉE</span>
                        <span className="block text-sm font-bold text-foreground">
                            {entrySpreadPct}%
                        </span>
                    </div>
                </div>
                <div className="h-[250px] flex items-center justify-center text-muted-foreground text-sm border border-dashed border-border rounded-lg">
                    📊 Graphique à venir — nécessite la base de données
                </div>
            </div>
        </>
    );
}
