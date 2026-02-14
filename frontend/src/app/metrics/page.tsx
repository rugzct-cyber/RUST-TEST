"use client";

import { useEffect, useRef, useState, useMemo, useCallback } from "react";
import {
    LineChart,
    Line,
    XAxis,
    YAxis,
    CartesianGrid,
    Tooltip,
    ResponsiveContainer,
    Legend,
} from "recharts";
import { useWS } from "@/components/providers";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
    EXCHANGES,
    SYMBOLS,
    EXCHANGE_LABELS,
    EXCHANGE_COLORS,
    MAX_CHART_POINTS,
    type Symbol,
} from "@/lib/constants";
import type { PriceData } from "@/lib/types";

// =============================================================================
// Types for chart data
// =============================================================================

interface SpreadPoint {
    time: number;
    label: string;
    [key: string]: number | string; // dynamic keys per exchange
}

// =============================================================================
// Metrics page — spread charts per symbol
// =============================================================================

export default function MetricsPage() {
    const { prices } = useWS();
    const [activeSymbol, setActiveSymbol] = useState<Symbol>("BTC");

    // Ring buffer of spread data points per symbol
    const bufferRef = useRef<Record<string, SpreadPoint[]>>(
        Object.fromEntries(SYMBOLS.map((s) => [s, []]))
    );
    const [chartData, setChartData] = useState<SpreadPoint[]>([]);

    // Accumulate spread data from price ticks
    const accumulateSpread = useCallback(
        (priceMap: Map<string, PriceData>) => {
            const now = Date.now();

            for (const symbol of SYMBOLS) {
                const point: SpreadPoint = {
                    time: now,
                    label: new Date(now).toLocaleTimeString(),
                };

                let hasData = false;
                for (const exchange of EXCHANGES) {
                    const key = `${exchange}:${symbol}`;
                    const p = priceMap.get(key);
                    if (p && p.bid > 0) {
                        const spread = ((p.ask - p.bid) / p.bid) * 100;
                        point[exchange] = Number(spread.toFixed(6));
                        hasData = true;
                    }
                }

                if (hasData) {
                    const buf = bufferRef.current[symbol];
                    buf.push(point);
                    // Ring buffer — cap at MAX_CHART_POINTS
                    if (buf.length > MAX_CHART_POINTS) {
                        bufferRef.current[symbol] = buf.slice(-MAX_CHART_POINTS);
                    }
                }
            }

            // Update visible chart data for active symbol
            setChartData([...bufferRef.current[activeSymbol]]);
        },
        [activeSymbol]
    );

    // Throttled accumulation (runs every 500ms to avoid chart re-render spam)
    useEffect(() => {
        const interval = setInterval(() => {
            accumulateSpread(prices);
        }, 500);
        return () => clearInterval(interval);
    }, [prices, accumulateSpread]);

    // Update chart when switching tabs
    useEffect(() => {
        setChartData([...bufferRef.current[activeSymbol]]);
    }, [activeSymbol]);

    // Compute current stats
    const stats = useMemo(() => {
        const result: Record<string, { spread: number; exchange: string }> = {};
        for (const exchange of EXCHANGES) {
            const key = `${exchange}:${activeSymbol}`;
            const p = prices.get(key);
            if (p && p.bid > 0) {
                result[exchange] = {
                    spread: ((p.ask - p.bid) / p.bid) * 100,
                    exchange,
                };
            }
        }
        return result;
    }, [prices, activeSymbol]);

    return (
        <div className="space-y-6">
            <div className="flex items-center gap-4">
                <h1 className="text-2xl font-bold tracking-tight">Metrics</h1>
                <Badge variant="secondary" className="font-mono text-xs">
                    {chartData.length} data points
                </Badge>
            </div>

            <Tabs value={activeSymbol} onValueChange={(v) => setActiveSymbol(v as Symbol)}>
                <TabsList>
                    {SYMBOLS.map((symbol) => (
                        <TabsTrigger key={symbol} value={symbol} className="font-mono">
                            {symbol}
                        </TabsTrigger>
                    ))}
                </TabsList>

                {SYMBOLS.map((symbol) => (
                    <TabsContent key={symbol} value={symbol} className="space-y-4">
                        {/* Spread Chart */}
                        <Card className="border-border/30 bg-card/50 backdrop-blur-sm">
                            <CardHeader className="pb-2">
                                <CardTitle className="text-sm font-medium text-muted-foreground">
                                    {symbol} Spread % by Exchange
                                </CardTitle>
                            </CardHeader>
                            <CardContent>
                                <div className="h-[400px] w-full">
                                    <ResponsiveContainer width="100%" height="100%">
                                        <LineChart data={chartData}>
                                            <CartesianGrid
                                                strokeDasharray="3 3"
                                                stroke="rgba(255,255,255,0.06)"
                                            />
                                            <XAxis
                                                dataKey="label"
                                                stroke="#888"
                                                fontSize={11}
                                                tickLine={false}
                                                tick={{ fill: "#888" }}
                                            />
                                            <YAxis
                                                stroke="#888"
                                                fontSize={11}
                                                tickLine={false}
                                                tick={{ fill: "#888" }}
                                                tickFormatter={(v: number) => `${v.toFixed(3)}%`}
                                            />
                                            <Tooltip
                                                contentStyle={{
                                                    backgroundColor: "rgba(15, 15, 35, 0.95)",
                                                    border: "1px solid rgba(255,255,255,0.1)",
                                                    borderRadius: "8px",
                                                    fontFamily: "var(--font-mono)",
                                                    fontSize: "12px",
                                                }}
                                                labelStyle={{ color: "#888" }}
                                                formatter={(value: number | string | Array<number | string> | undefined) => {
                                                    if (typeof value === "number") return [`${value.toFixed(6)}%`, ""];
                                                    return [String(value ?? "—"), ""];
                                                }}
                                            />
                                            <Legend
                                                wrapperStyle={{ fontSize: "12px", fontFamily: "var(--font-mono)" }}
                                            />
                                            {EXCHANGES.map((exchange) => (
                                                <Line
                                                    key={exchange}
                                                    type="monotone"
                                                    dataKey={exchange}
                                                    name={EXCHANGE_LABELS[exchange]}
                                                    stroke={EXCHANGE_COLORS[exchange]}
                                                    strokeWidth={2}
                                                    dot={false}
                                                    connectNulls
                                                    isAnimationActive={false}
                                                />
                                            ))}
                                        </LineChart>
                                    </ResponsiveContainer>
                                </div>
                            </CardContent>
                        </Card>

                        {/* Current spread stats */}
                        <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
                            {EXCHANGES.map((exchange) => {
                                const stat = stats[exchange];
                                return (
                                    <Card
                                        key={exchange}
                                        className="border-border/30 bg-card/50 backdrop-blur-sm"
                                    >
                                        <CardHeader className="pb-2">
                                            <CardTitle className="text-sm font-medium text-muted-foreground">
                                                {EXCHANGE_LABELS[exchange]}
                                            </CardTitle>
                                        </CardHeader>
                                        <CardContent>
                                            <p className="font-mono text-2xl font-bold tabular-nums">
                                                {stat ? `${stat.spread.toFixed(4)}%` : "—"}
                                            </p>
                                            <p className="text-xs text-muted-foreground">
                                                Current spread
                                            </p>
                                        </CardContent>
                                    </Card>
                                );
                            })}
                        </div>
                    </TabsContent>
                ))}
            </Tabs>
        </div>
    );
}
