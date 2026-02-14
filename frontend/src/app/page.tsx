"use client";

import { useMemo, useState, useCallback } from "react";
import { useWS } from "@/components/providers";
import { ExchangeSidebar } from "@/components/exchange-sidebar";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { PriceData } from "@/lib/types";

// =============================================================================
// Helpers
// =============================================================================

/** Format price based on magnitude */
function formatPrice(price: number): string {
  if (price === 0) return "—";
  if (price >= 10_000) return price.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  if (price >= 100) return price.toFixed(2);
  if (price >= 1) return price.toFixed(4);
  return price.toFixed(6);
}

/** Compute arbitrage spread: buy at bestAsk, sell at bestBid → positive = profitable */
function computeSpread(bestBid: number, bestAsk: number): number | null {
  if (bestBid <= 0 || bestAsk <= 0) return null;
  return ((bestBid - bestAsk) / bestAsk) * 100;
}

// =============================================================================
// Dashboard page — arbi-v5 style table layout
// =============================================================================

export default function DashboardPage() {
  const { prices, status } = useWS();

  // ---- Discover exchanges and symbols dynamically from WS data ----
  const { allExchanges, allSymbols } = useMemo(() => {
    const exchanges = new Set<string>();
    const symbols = new Set<string>();
    for (const [key] of prices) {
      const [exchange, symbol] = key.split(":");
      exchanges.add(exchange);
      symbols.add(symbol);
    }
    return {
      allExchanges: Array.from(exchanges).sort(),
      allSymbols: Array.from(symbols).sort(),
    };
  }, [prices]);

  // ---- Exchange filter state ----
  const [selectedExchanges, setSelectedExchanges] = useState<Set<string>>(
    new Set()
  );

  // Auto-select all new exchanges when first discovered
  useMemo(() => {
    setSelectedExchanges((prev) => {
      const next = new Set(prev);
      for (const e of allExchanges) {
        if (!prev.has(e) && prev.size === 0) {
          // First load: select all
          return new Set(allExchanges);
        }
        if (!prev.has(e)) {
          next.add(e);
        }
      }
      return next.size !== prev.size ? next : prev;
    });
  }, [allExchanges]);

  const handleToggle = useCallback((exchange: string) => {
    setSelectedExchanges((prev) => {
      const next = new Set(prev);
      if (next.has(exchange)) next.delete(exchange);
      else next.add(exchange);
      return next;
    });
  }, []);

  const handleSelectAll = useCallback(() => {
    setSelectedExchanges(new Set(allExchanges));
  }, [allExchanges]);

  const handleDeselectAll = useCallback(() => {
    setSelectedExchanges(new Set());
  }, []);

  // ---- Visible exchanges (filtered) ----
  const visibleExchanges = useMemo(
    () => allExchanges.filter((e) => selectedExchanges.has(e)),
    [allExchanges, selectedExchanges]
  );

  // ---- Per-symbol: compute best bid, best ask, spread, strategy ----
  const symbolData = useMemo(() => {
    return allSymbols.map((symbol) => {
      let bestBid = -Infinity;
      let bestAsk = Infinity;
      let bestBidExchange = "";
      let bestAskExchange = "";
      let latestPrice = 0;

      const exchangePrices: Record<string, PriceData> = {};

      for (const exchange of allExchanges) {
        const key = `${exchange}:${symbol}`;
        const p = prices.get(key);
        if (!p) continue;
        exchangePrices[exchange] = p;

        if (p.bid > bestBid) {
          bestBid = p.bid;
          bestBidExchange = exchange;
        }
        if (p.ask < bestAsk && p.ask > 0) {
          bestAsk = p.ask;
          bestAskExchange = exchange;
        }
        if (p.bid > 0) latestPrice = p.bid;
      }

      const spread = bestBid > 0 && bestAsk < Infinity
        ? computeSpread(bestBid, bestAsk)
        : null;

      return {
        symbol,
        latestPrice,
        bestBid: bestBid > 0 ? bestBid : 0,
        bestAsk: bestAsk < Infinity ? bestAsk : 0,
        bestBidExchange,
        bestAskExchange,
        spread,
        // Strategy: LONG on cheapest ask exchange, SHORT on highest bid exchange
        longExchange: bestAskExchange,
        shortExchange: bestBidExchange,
        exchangePrices,
      };
    });
  }, [allSymbols, allExchanges, prices]);

  // ---- Search filter for pairs ----
  const [pairSearch, setPairSearch] = useState("");
  const filteredSymbols = useMemo(
    () =>
      symbolData.filter((s) =>
        s.symbol.toLowerCase().includes(pairSearch.toLowerCase())
      ),
    [symbolData, pairSearch]
  );

  // ---- Empty state ----
  if (prices.size === 0) {
    return (
      <div className="flex h-[80vh] flex-col items-center justify-center gap-3 text-muted-foreground">
        <span className="text-4xl">📡</span>
        <p className="text-lg font-medium">Waiting for price data…</p>
        <p className="text-sm">
          Connect the Rust backend on{" "}
          <code className="rounded bg-muted px-1.5 py-0.5 text-xs">
            ws://localhost:8080/ws
          </code>
        </p>
        <Badge variant="secondary" className="mt-2 capitalize">
          {status}
        </Badge>
      </div>
    );
  }

  return (
    <div className="flex h-[calc(100vh-3.5rem)]">
      {/* ---- Sidebar ---- */}
      <ExchangeSidebar
        allExchanges={allExchanges}
        selectedExchanges={selectedExchanges}
        onToggle={handleToggle}
        onSelectAll={handleSelectAll}
        onDeselectAll={handleDeselectAll}
      />

      {/* ---- Main table area ---- */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Toolbar */}
        <div className="flex items-center gap-3 border-b border-border/30 bg-card/30 px-4 py-2 backdrop-blur-sm">
          <input
            type="text"
            value={pairSearch}
            onChange={(e) => setPairSearch(e.target.value)}
            placeholder="Filter pairs…"
            className="w-48 rounded-md border border-border/30 bg-background/50 px-3 py-1 text-sm text-foreground placeholder:text-muted-foreground focus:border-emerald-500/50 focus:outline-none"
          />
          <Badge variant="secondary" className="font-mono text-xs">
            {filteredSymbols.length} pairs
          </Badge>
          <Badge variant="secondary" className="font-mono text-xs">
            {visibleExchanges.length} exchanges
          </Badge>
        </div>

        {/* Scrollable table */}
        <div className="flex-1 overflow-auto">
          <table className="w-full min-w-max border-collapse text-sm">
            {/* Header */}
            <thead className="sticky top-0 z-10 bg-background/95 backdrop-blur-sm">
              <tr className="border-b border-border/30">
                {/* Frozen columns */}
                <th className="sticky left-0 z-20 bg-background/95 px-3 py-2.5 text-left text-xs font-semibold uppercase tracking-wider text-muted-foreground backdrop-blur-sm">
                  Pair
                </th>
                <th className="sticky left-[100px] z-20 bg-background/95 px-3 py-2.5 text-left text-xs font-semibold uppercase tracking-wider text-muted-foreground backdrop-blur-sm min-w-[180px]">
                  Spread & Strategy
                </th>
                {/* Exchange columns */}
                {visibleExchanges.map((exchange) => (
                  <th
                    key={exchange}
                    className="px-3 py-2.5 text-center text-xs font-semibold uppercase tracking-wider text-muted-foreground min-w-[120px]"
                  >
                    {exchange}
                  </th>
                ))}
              </tr>
            </thead>

            {/* Body */}
            <tbody className="divide-y divide-border/20">
              {filteredSymbols.map((row) => (
                <tr
                  key={row.symbol}
                  className="transition-colors hover:bg-accent/30"
                >
                  {/* Pair name + price */}
                  <td className="sticky left-0 z-10 bg-background/90 px-3 py-2.5 backdrop-blur-sm">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-foreground">
                        {row.symbol}
                      </span>
                      {row.latestPrice > 0 && (
                        <span className="font-mono text-xs text-muted-foreground tabular-nums">
                          {formatPrice(row.latestPrice)}
                        </span>
                      )}
                    </div>
                  </td>

                  {/* Spread & Strategy */}
                  <td className="sticky left-[100px] z-10 bg-background/90 px-3 py-2.5 backdrop-blur-sm">
                    <div className="space-y-0.5">
                      {row.spread !== null ? (
                        <span
                          className={cn(
                            "font-mono text-sm font-bold tabular-nums",
                            row.spread >= 0.5
                              ? "text-emerald-400"
                              : row.spread >= 0.1
                                ? "text-amber-400"
                                : "text-muted-foreground"
                          )}
                        >
                          {row.spread.toFixed(4)}%
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">—</span>
                      )}
                      {row.longExchange && row.shortExchange && (
                        <div className="flex items-center gap-1 text-[10px]">
                          <span className="rounded bg-emerald-500/15 px-1 py-0.5 text-emerald-400">
                            LONG
                          </span>
                          <span className="text-muted-foreground">
                            {row.longExchange}
                          </span>
                          <span className="rounded bg-red-500/15 px-1 py-0.5 text-red-400">
                            SHORT
                          </span>
                          <span className="text-muted-foreground">
                            {row.shortExchange}
                          </span>
                        </div>
                      )}
                    </div>
                  </td>

                  {/* Exchange price cells */}
                  {visibleExchanges.map((exchange) => {
                    const p = row.exchangePrices[exchange];
                    if (!p) {
                      return (
                        <td
                          key={exchange}
                          className="px-3 py-2.5 text-center text-xs text-muted-foreground/40"
                        >
                          —
                        </td>
                      );
                    }

                    const isBestBid = exchange === row.bestBidExchange;
                    const isBestAsk = exchange === row.bestAskExchange;

                    return (
                      <td key={exchange} className="px-3 py-2.5">
                        <div className="flex flex-col items-center gap-0.5">
                          {/* Bid */}
                          <span
                            className={cn(
                              "font-mono text-xs tabular-nums",
                              isBestBid
                                ? "font-bold text-emerald-400"
                                : "text-foreground/80"
                            )}
                          >
                            {formatPrice(p.bid)}
                          </span>
                          {/* Ask */}
                          <span
                            className={cn(
                              "font-mono text-xs tabular-nums",
                              isBestAsk
                                ? "font-bold text-red-400"
                                : "text-foreground/50"
                            )}
                          >
                            {formatPrice(p.ask)}
                          </span>
                        </div>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
