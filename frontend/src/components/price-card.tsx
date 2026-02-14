"use client";

import { useMemo } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { PriceData } from "@/lib/types";
import { STALE_THRESHOLD_MS, WARN_THRESHOLD_MS } from "@/lib/constants";

// =============================================================================
// PriceCard — single exchange × symbol tile
// =============================================================================

interface PriceCardProps {
    data: PriceData | undefined;
    exchange: string;
    symbol: string;
    isBestBid?: boolean;
    isBestAsk?: boolean;
}

export function PriceCard({
    data,
    exchange,
    symbol,
    isBestBid = false,
    isBestAsk = false,
}: PriceCardProps) {
    const freshness = useMemo(() => {
        if (!data) return "stale";
        const age = Date.now() - data.timestamp_ms;
        if (age < WARN_THRESHOLD_MS) return "fresh";
        if (age < STALE_THRESHOLD_MS) return "warn";
        return "stale";
    }, [data]);

    const spread = useMemo(() => {
        if (!data || data.bid === 0) return null;
        return ((data.ask - data.bid) / data.bid) * 100;
    }, [data]);

    const formatPrice = (price: number) => {
        if (price >= 1000) return price.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
        if (price >= 1) return price.toFixed(4);
        return price.toFixed(6);
    };

    return (
        <Card
            className={cn(
                "relative overflow-hidden transition-all duration-300",
                "border-border/30 bg-card/50 backdrop-blur-sm",
                "hover:border-border/60 hover:shadow-lg hover:shadow-emerald-500/5",
                freshness === "stale" && "opacity-40",
                freshness === "warn" && "border-amber-500/30"
            )}
        >
            {/* Best bid/ask glow indicator */}
            {(isBestBid || isBestAsk) && (
                <div
                    className={cn(
                        "absolute inset-x-0 top-0 h-0.5",
                        isBestBid && isBestAsk
                            ? "bg-gradient-to-r from-emerald-500 to-cyan-500"
                            : isBestBid
                                ? "bg-emerald-500"
                                : "bg-cyan-500"
                    )}
                />
            )}

            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">
                    {exchange}
                </CardTitle>
                <Badge
                    variant="outline"
                    className={cn(
                        "text-xs",
                        freshness === "fresh" && "border-emerald-500/50 text-emerald-400",
                        freshness === "warn" && "border-amber-500/50 text-amber-400",
                        freshness === "stale" && "border-muted text-muted-foreground"
                    )}
                >
                    {symbol}
                </Badge>
            </CardHeader>

            <CardContent className="space-y-2">
                {data ? (
                    <>
                        {/* Bid / Ask */}
                        <div className="grid grid-cols-2 gap-2">
                            <div>
                                <p className="text-xs text-muted-foreground">Bid</p>
                                <p
                                    className={cn(
                                        "font-mono text-base font-semibold tabular-nums",
                                        isBestBid ? "text-emerald-400" : "text-foreground"
                                    )}
                                >
                                    {formatPrice(data.bid)}
                                </p>
                            </div>
                            <div className="text-right">
                                <p className="text-xs text-muted-foreground">Ask</p>
                                <p
                                    className={cn(
                                        "font-mono text-base font-semibold tabular-nums",
                                        isBestAsk ? "text-cyan-400" : "text-foreground"
                                    )}
                                >
                                    {formatPrice(data.ask)}
                                </p>
                            </div>
                        </div>

                        {/* Spread */}
                        {spread !== null && (
                            <div className="flex items-center justify-between rounded-md bg-muted/30 px-2 py-1">
                                <span className="text-xs text-muted-foreground">Spread</span>
                                <span
                                    className={cn(
                                        "font-mono text-xs tabular-nums",
                                        spread < 0.05
                                            ? "text-emerald-400"
                                            : spread < 0.2
                                                ? "text-amber-400"
                                                : "text-red-400"
                                    )}
                                >
                                    {spread.toFixed(4)}%
                                </span>
                            </div>
                        )}
                    </>
                ) : (
                    <div className="flex h-16 items-center justify-center text-sm text-muted-foreground">
                        Waiting for data…
                    </div>
                )}
            </CardContent>
        </Card>
    );
}
