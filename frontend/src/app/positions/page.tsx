"use client";

import { useState, useEffect, useMemo, useCallback } from "react";
import { useWS } from "@/components/providers";
import { AddPositionForm } from "@/components/positions/add-position-form";
import { PositionDetail } from "@/components/positions/position-detail";
import type { Position, ExitSpreadData, PriceData } from "@/lib/types";
import { EXCHANGES } from "@/lib/constants";

// =============================================================================
// localStorage helpers
// =============================================================================

const STORAGE_KEY = "arbi-positions";

function loadPositions(): Position[] {
    if (typeof window === "undefined") return [];
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        return raw ? JSON.parse(raw) : [];
    } catch { return []; }
}

function savePositions(positions: Position[]) {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify(positions)); } catch { /* ignore */ }
}

// =============================================================================
// Exit spread computation
// =============================================================================

function computeExitSpread(
    position: Position,
    prices: Map<string, PriceData>
): ExitSpreadData | null {
    // We need the long exchange bid (to sell) and the short exchange ask (to buy back)
    const longKey = `${position.longExchange}:${position.token}`;
    const shortKey = `${position.shortExchange}:${position.token}`;

    const longPrice = prices.get(longKey);
    const shortPrice = prices.get(shortKey);

    if (!longPrice || !shortPrice) return null;
    if (longPrice.bid <= 0 || shortPrice.ask <= 0) return null;

    const exitSpreadDollar = longPrice.bid - shortPrice.ask;
    const exitSpread = (exitSpreadDollar / shortPrice.ask) * 100;

    // PnL = (exit spread - entry spread dollar) × tokens
    const entrySpreadDollar = position.entryPriceShort - position.entryPriceLong;
    const pnl = (exitSpreadDollar - entrySpreadDollar) * position.tokenAmount;

    return {
        exitSpread,
        exitSpreadDollar,
        longBid: longPrice.bid,
        longAsk: longPrice.ask,
        shortBid: shortPrice.bid,
        shortAsk: shortPrice.ask,
        isInProfit: pnl >= 0,
        pnl,
    };
}

// =============================================================================
// Page component
// =============================================================================

export default function PositionsPage() {
    const { prices } = useWS();
    const [positions, setPositions] = useState<Position[]>([]);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [mounted, setMounted] = useState(false);

    // Load from localStorage on mount
    useEffect(() => {
        setPositions(loadPositions());
        setMounted(true);
    }, []);

    // Persist on change
    useEffect(() => {
        if (mounted) savePositions(positions);
    }, [positions, mounted]);

    // Derived — unique tokens and exchanges from WS data for autocomplete
    const availableTokens = useMemo(() => {
        const tokens = new Set<string>();
        for (const key of prices.keys()) {
            const symbol = key.split(":")[1];
            if (symbol) tokens.add(symbol);
        }
        return Array.from(tokens).sort();
    }, [prices]);

    const availableExchanges = useMemo(() => {
        const exs = new Set<string>(EXCHANGES);
        for (const key of prices.keys()) {
            const exchange = key.split(":")[0];
            if (exchange) exs.add(exchange);
        }
        return Array.from(exs).sort();
    }, [prices]);

    // Handlers
    const handleAdd = useCallback((pos: Position) => {
        setPositions((prev) => [pos, ...prev]);
        setSelectedId(pos.id);
    }, []);

    const handleDelete = useCallback((id: string) => {
        setPositions((prev) => prev.filter((p) => p.id !== id));
        setSelectedId((prev) => (prev === id ? null : prev));
    }, []);

    const handleUpdate = useCallback((updated: Position) => {
        setPositions((prev) => prev.map((p) => (p.id === updated.id ? updated : p)));
    }, []);

    const selectedPosition = positions.find((p) => p.id === selectedId) ?? null;

    // Live exit spread for selected position
    const exitSpreadData = useMemo(() => {
        if (!selectedPosition) return null;
        return computeExitSpread(selectedPosition, prices);
    }, [selectedPosition, prices]);

    // Current PnL
    const currentPnL = exitSpreadData?.pnl ?? null;

    return (
        <div className="flex h-[calc(100vh-3.5rem)]">
            {/* ─── LEFT SIDEBAR ─────────────────────────────────── */}
            <aside className="w-[340px] flex-shrink-0 border-r border-border/40 bg-background/60 flex flex-col">
                {/* Add form */}
                <div className="p-4 border-b border-border/40">
                    <AddPositionForm
                        availableTokens={availableTokens}
                        availableExchanges={availableExchanges}
                        onAdd={handleAdd}
                    />
                </div>

                {/* Position list */}
                <div className="flex-1 overflow-y-auto p-4 space-y-2">
                    <div className="flex items-center justify-between mb-2">
                        <h3 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide">
                            Positions Ouvertes ({positions.length})
                        </h3>
                    </div>

                    {positions.length === 0 && (
                        <div className="text-center text-muted-foreground text-sm py-8">
                            Aucune position
                        </div>
                    )}

                    {positions.map((pos) => {
                        const posSpread = computeExitSpread(pos, prices);
                        const isSelected = pos.id === selectedId;

                        return (
                            <div
                                key={pos.id}
                                className={`
                                    rounded-lg border p-3 cursor-pointer transition-all duration-200
                                    ${isSelected
                                        ? "border-primary bg-primary/10 shadow-[inset_0_0_0_1px_var(--primary)]"
                                        : "border-border bg-card hover:border-border hover:bg-card/80"
                                    }
                                `}
                                onClick={() => setSelectedId(pos.id)}
                            >
                                <div className="flex items-center justify-between mb-1">
                                    <span className="font-semibold text-foreground text-sm">
                                        {pos.token}
                                    </span>
                                    <button
                                        className="w-6 h-6 flex items-center justify-center rounded-md text-muted-foreground hover:text-destructive hover:bg-destructive/10 transition-colors"
                                        onClick={(e) => { e.stopPropagation(); handleDelete(pos.id); }}
                                        title="Supprimer"
                                    >
                                        ✕
                                    </button>
                                </div>

                                <div className="text-[0.625rem] text-muted-foreground uppercase tracking-wide mb-1">
                                    LONG {pos.longExchange.toUpperCase()} / SHORT {pos.shortExchange.toUpperCase()}
                                </div>

                                <div className="flex items-center justify-between text-xs">
                                    <span className="text-muted-foreground">
                                        {pos.tokenAmount} tokens
                                    </span>
                                    <span className={`font-mono font-semibold ${(posSpread?.pnl ?? 0) >= 0 ? "text-green-400" : "text-red-400"}`}>
                                        {posSpread ? `$${posSpread.pnl.toFixed(2)}` : "—"}
                                    </span>
                                </div>
                            </div>
                        );
                    })}
                </div>
            </aside>

            {/* ─── MAIN CONTENT ─────────────────────────────────── */}
            <section className="flex-1 overflow-y-auto p-6">
                {selectedPosition ? (
                    <PositionDetail
                        position={selectedPosition}
                        exitSpreadData={exitSpreadData}
                        currentPnL={currentPnL}
                        onUpdatePosition={handleUpdate}
                    />
                ) : (
                    <div className="h-full flex items-center justify-center">
                        <div className="text-center text-muted-foreground">
                            <span className="block text-4xl mb-4">📈</span>
                            <h3 className="text-lg font-medium mb-1">👈 Sélectionne une position</h3>
                            <p className="text-sm">Clique sur une position à gauche pour voir le spread de sortie en temps réel</p>
                        </div>
                    </div>
                )}
            </section>
        </div>
    );
}
