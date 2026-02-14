"use client";

import { useMemo, useState, useCallback, useEffect, useRef } from "react";
import { useWS } from "@/components/providers";
import { ExchangeSidebar } from "@/components/exchange-sidebar";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { PriceData } from "@/lib/types";

// =============================================================================
// Helpers
// =============================================================================

function formatPrice(price: number): string {
  if (price === 0) return "—";
  if (price >= 10_000)
    return price.toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    });
  if (price >= 100) return price.toFixed(2);
  if (price >= 1) return price.toFixed(4);
  return price.toFixed(6);
}

function computeSpread(bestBid: number, bestAsk: number): number | null {
  if (bestBid <= 0 || bestAsk <= 0) return null;
  return ((bestBid - bestAsk) / bestAsk) * 100;
}

// =============================================================================
// Types
// =============================================================================

interface SymbolRow {
  symbol: string;
  latestPrice: number;
  bestBid: number;
  bestAsk: number;
  bestBidExchange: string;
  bestAskExchange: string;
  spread: number | null;
  longExchange: string;
  shortExchange: string;
  exchangePrices: Record<string, PriceData>;
}

interface SpreadHistoryPoint {
  time: number;
  spread: number;
}

// =============================================================================
// SpreadChart — original arbi-v5 style
// =============================================================================

function SpreadChartPanel({
  symbol,
  longExchange,
  shortExchange,
  spread,
  onClose,
  colSpan,
}: {
  symbol: string;
  longExchange: string;
  shortExchange: string;
  spread: number | null;
  onClose: () => void;
  colSpan: number;
}) {
  const historyRef = useRef<SpreadHistoryPoint[]>([]);
  const [history, setHistory] = useState<SpreadHistoryPoint[]>([]);
  const [range, setRange] = useState<"1M" | "5M" | "30M" | "1H">("5M");

  useEffect(() => {
    if (spread === null) return;
    const point = { time: Date.now(), spread };
    historyRef.current.push(point);
    if (historyRef.current.length > 3600) historyRef.current.shift();
    setHistory([...historyRef.current]);
  }, [spread]);

  const rangeMs = { "1M": 60_000, "5M": 300_000, "30M": 1_800_000, "1H": 3_600_000 };
  const filtered = history.filter((p) => p.time >= Date.now() - rangeMs[range]);

  const spreads = filtered.map((p) => p.spread);
  const avg = spreads.length > 0 ? spreads.reduce((a, b) => a + b, 0) / spreads.length : 0;
  const min = spreads.length > 0 ? Math.min(...spreads) : 0;
  const max = spreads.length > 0 ? Math.max(...spreads) : 0;

  const chartWidth = 900;
  const chartHeight = 200;
  const padding = { top: 20, right: 40, bottom: 30, left: 60 };
  const innerW = chartWidth - padding.left - padding.right;
  const innerH = chartHeight - padding.top - padding.bottom;

  const yMin = spreads.length > 0 ? Math.min(...spreads) * 0.9 : 0;
  const yMax = spreads.length > 0 ? Math.max(...spreads) * 1.1 : 1;
  const yRange = yMax - yMin || 0.01;

  const timeMin = filtered.length > 0 ? filtered[0].time : Date.now() - rangeMs[range];
  const timeMax = filtered.length > 0 ? filtered[filtered.length - 1].time : Date.now();
  const timeRange = timeMax - timeMin || 1;

  const points = filtered
    .map((p) => {
      const x = padding.left + ((p.time - timeMin) / timeRange) * innerW;
      const y = padding.top + (1 - (p.spread - yMin) / yRange) * innerH;
      return `${x},${y}`;
    })
    .join(" ");

  const avgY = padding.top + (1 - (avg - yMin) / yRange) * innerH;

  const areaPoints = filtered.length >= 2
    ? `${padding.left},${padding.top + innerH} ${points} ${padding.left + ((filtered[filtered.length - 1].time - timeMin) / timeRange) * innerW},${padding.top + innerH}`
    : "";

  return (
    <tr>
      <td colSpan={colSpan} style={{ padding: 0, borderBottom: "none" }}>
        {/* Chart container — matches SpreadChart.module.css exactly */}
        <div
          className="animate-slideIn"
          style={{
            background: "linear-gradient(180deg, rgba(15, 23, 42, 0.95) 0%, rgba(15, 23, 42, 0.8) 100%)",
            border: "1px solid rgba(59, 130, 246, 0.3)",
            borderRadius: 8,
            padding: 16,
            margin: "8px 12px",
          }}
        >
          {/* Header */}
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 16 }}>
            <div style={{ display: "flex", alignItems: "baseline", gap: 12 }}>
              <span style={{ fontSize: 18, fontWeight: 700, color: "#f8fafc" }}>{symbol}</span>
              <span style={{ fontSize: 12, color: "#64748b", textTransform: "uppercase" }}>
                {longExchange} vs {shortExchange}
              </span>
            </div>
            <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
              {(["1M", "5M", "30M", "1H"] as const).map((r) => (
                <button
                  key={r}
                  onClick={() => setRange(r)}
                  style={{
                    padding: "6px 12px",
                    fontSize: 12,
                    fontWeight: 500,
                    background: range === r ? "rgba(59, 130, 246, 0.3)" : "rgba(51, 65, 85, 0.5)",
                    border: `1px solid ${range === r ? "rgba(59, 130, 246, 0.5)" : "rgba(71, 85, 105, 0.5)"}`,
                    borderRadius: 4,
                    color: range === r ? "#3b82f6" : "#94a3b8",
                    cursor: "pointer",
                    transition: "background 0.15s, border-color 0.15s, color 0.15s",
                  }}
                >
                  {r}
                </button>
              ))}
              <button
                onClick={onClose}
                style={{
                  marginLeft: 12,
                  padding: "6px 10px",
                  fontSize: 14,
                  background: "transparent",
                  border: "1px solid rgba(239, 68, 68, 0.3)",
                  borderRadius: 4,
                  color: "#ef4444",
                  cursor: "pointer",
                  transition: "background 0.15s",
                }}
                title="Close"
              >
                ✕
              </button>
            </div>
          </div>

          {/* Stats bar */}
          <div
            style={{
              display: "flex",
              gap: 24,
              padding: "12px 16px",
              background: "rgba(30, 41, 59, 0.5)",
              borderRadius: 6,
              marginBottom: 16,
            }}
          >
            <StatCard label="CURRENT" value={spread !== null ? `${spread.toFixed(3)}%` : "—"} color="#22c55e" />
            <StatCard label={`AVG (${range})`} value={`${avg.toFixed(3)}%`} color="#f8fafc" />
            <StatCard label="MIN" value={`${min.toFixed(3)}%`} color="#f8fafc" />
            <StatCard label="MAX" value={`${max.toFixed(3)}%`} color="#f8fafc" />
            {spreads.length > 0 && (
              <StatCard
                label={`PCTLE (${range})`}
                value={`${Math.round((spreads.filter((s) => s <= (spread ?? 0)).length / spreads.length) * 100)}%`}
                color="#f8fafc"
              />
            )}
          </div>

          {/* SVG Chart */}
          <div style={{ position: "relative", minHeight: 200 }}>
            {filtered.length < 2 ? (
              <div style={{ display: "flex", alignItems: "center", justifyContent: "center", minHeight: 200, color: "#64748b", fontSize: 14 }}>
                Collecting data… ({filtered.length} points)
              </div>
            ) : (
              <svg viewBox={`0 0 ${chartWidth} ${chartHeight}`} style={{ width: "100%" }} preserveAspectRatio="xMidYMid meet">
                <defs>
                  <linearGradient id={`area-gradient-${symbol}`} x1="0" x2="0" y1="0" y2="1">
                    <stop offset="0%" stopColor="#3b82f6" stopOpacity="0.3" />
                    <stop offset="100%" stopColor="#3b82f6" stopOpacity="0" />
                  </linearGradient>
                </defs>

                {/* Grid lines */}
                {[0, 0.25, 0.5, 0.75, 1].map((pct) => {
                  const y = padding.top + pct * innerH;
                  const val = yMax - pct * yRange;
                  return (
                    <g key={pct}>
                      <line
                        x1={padding.left} y1={y}
                        x2={chartWidth - padding.right} y2={y}
                        stroke="rgba(255,255,255,0.05)" strokeDasharray="3,6"
                      />
                      <text x={padding.left - 10} y={y + 3} textAnchor="end" fill="#64748b" fontSize="10" fontFamily="monospace">
                        {val.toFixed(3)}%
                      </text>
                    </g>
                  );
                })}

                {/* Average reference line */}
                <line
                  x1={padding.left} y1={avgY}
                  x2={chartWidth - padding.right} y2={avgY}
                  stroke="#3b82f6" strokeDasharray="6,4" strokeWidth={1} opacity={0.5}
                />
                <text x={chartWidth - padding.right + 5} y={avgY + 3} fill="#3b82f6" fontSize="9" opacity={0.7} fontFamily="monospace">
                  Avg
                </text>

                {/* Area fill */}
                {areaPoints && (
                  <polygon points={areaPoints} fill={`url(#area-gradient-${symbol})`} />
                )}

                {/* Spread line */}
                <polyline
                  points={points}
                  fill="none"
                  stroke="#3b82f6"
                  strokeWidth={2}
                  strokeLinejoin="round"
                  strokeLinecap="round"
                />
              </svg>
            )}
          </div>
        </div>
      </td>
    </tr>
  );
}

function StatCard({ label, value, color }: { label: string; value: string; color: string }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: 10, color: "#64748b", textTransform: "uppercase", letterSpacing: "0.5px" }}>
        {label}
      </span>
      <span style={{ fontSize: 16, fontWeight: 600, color, fontFamily: "monospace" }}>
        {value}
      </span>
    </div>
  );
}

// =============================================================================
// Dashboard page
// =============================================================================

export default function DashboardPage() {
  const { prices, status } = useWS();

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

  const [selectedExchanges, setSelectedExchanges] = useState<Set<string>>(new Set());

  useMemo(() => {
    setSelectedExchanges((prev) => {
      const next = new Set(prev);
      for (const e of allExchanges) {
        if (!prev.has(e) && prev.size === 0) return new Set(allExchanges);
        if (!prev.has(e)) next.add(e);
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

  const handleSelectAll = useCallback(() => { setSelectedExchanges(new Set(allExchanges)); }, [allExchanges]);
  const handleDeselectAll = useCallback(() => { setSelectedExchanges(new Set()); }, []);

  const visibleExchanges = useMemo(
    () => allExchanges.filter((e) => selectedExchanges.has(e)),
    [allExchanges, selectedExchanges]
  );

  // ---- Favorites ----
  const [favorites, setFavorites] = useState<Set<string>>(new Set());
  const [showFavoritesOnly, setShowFavoritesOnly] = useState(false);

  useEffect(() => {
    const saved = localStorage.getItem("dashboard-favorites");
    if (saved) { try { setFavorites(new Set(JSON.parse(saved))); } catch { /* */ } }
  }, []);

  const handleFavoriteToggle = useCallback((symbol: string) => {
    setFavorites((prev) => {
      const next = new Set(prev);
      if (next.has(symbol)) next.delete(symbol);
      else next.add(symbol);
      localStorage.setItem("dashboard-favorites", JSON.stringify([...next]));
      return next;
    });
  }, []);

  // ---- Pinned exchange ----
  const [pinnedExchange, setPinnedExchange] = useState<string | null>(null);

  useEffect(() => {
    const saved = localStorage.getItem("dashboard-pinned-exchange");
    if (saved) setPinnedExchange(saved);
  }, []);

  const handlePinExchange = useCallback((exchangeId: string) => {
    setPinnedExchange((prev) => {
      const next = prev === exchangeId ? null : exchangeId;
      if (next) localStorage.setItem("dashboard-pinned-exchange", next);
      else localStorage.removeItem("dashboard-pinned-exchange");
      return next;
    });
  }, []);

  // ---- Sorting ----
  const [sortColumn, setSortColumn] = useState<string>("spread");
  const [sortDirection, setSortDirection] = useState<"asc" | "desc">("desc");

  const handleSort = useCallback((column: string) => {
    setSortColumn((prev) => {
      if (prev === column) {
        setSortDirection((d) => (d === "asc" ? "desc" : "asc"));
        return prev;
      }
      setSortDirection(column === "spread" ? "desc" : "asc");
      return column;
    });
  }, []);

  // ---- Expanded row ----
  const [expandedRow, setExpandedRow] = useState<string | null>(null);

  // ---- Search ----
  const [pairSearch, setPairSearch] = useState("");

  // ---- Data ----
  const symbolData = useMemo((): SymbolRow[] => {
    return allSymbols.map((symbol) => {
      const validPrices: { exchange: string; bid: number; ask: number }[] = [];
      const exchangePrices: Record<string, PriceData> = {};
      let latestPrice = 0;

      for (const exchange of allExchanges) {
        const key = `${exchange}:${symbol}`;
        const p = prices.get(key);
        if (!p) continue;
        exchangePrices[exchange] = p;
        if (p.bid > 0 && p.ask > 0) validPrices.push({ exchange, bid: p.bid, ask: p.ask });
        if (p.bid > 0) latestPrice = p.bid;
      }

      let bestBid = 0, bestAsk = 0, bestBidExchange = "", bestAskExchange = "";
      let spread: number | null = null;

      if (pinnedExchange && selectedExchanges.has(pinnedExchange)) {
        const pinnedPrice = validPrices.find((p) => p.exchange === pinnedExchange);
        if (pinnedPrice && validPrices.length >= 2) {
          let best = -Infinity;
          for (const other of validPrices) {
            if (other.exchange === pinnedExchange || !selectedExchanges.has(other.exchange)) continue;
            const s1 = ((pinnedPrice.bid - other.ask) / other.ask) * 100;
            if (s1 > best) { best = s1; bestBidExchange = pinnedExchange; bestAskExchange = other.exchange; bestBid = pinnedPrice.bid; bestAsk = other.ask; }
            const s2 = ((other.bid - pinnedPrice.ask) / pinnedPrice.ask) * 100;
            if (s2 > best) { best = s2; bestBidExchange = other.exchange; bestAskExchange = pinnedExchange; bestBid = other.bid; bestAsk = pinnedPrice.ask; }
          }
          if (best > -Infinity) spread = Math.round(best * 10000) / 10000;
        }
      } else {
        for (const p of validPrices) {
          if (p.bid > bestBid) { bestBid = p.bid; bestBidExchange = p.exchange; }
          if ((p.ask < bestAsk || bestAsk === 0) && p.ask > 0) { bestAsk = p.ask; bestAskExchange = p.exchange; }
        }
        if (bestBid > 0 && bestAsk > 0) { spread = computeSpread(bestBid, bestAsk); if (spread !== null) spread = Math.round(spread * 10000) / 10000; }
      }

      return { symbol, latestPrice, bestBid, bestAsk, bestBidExchange, bestAskExchange, spread, longExchange: bestAskExchange, shortExchange: bestBidExchange, exchangePrices };
    });
  }, [allSymbols, allExchanges, prices, pinnedExchange, selectedExchanges]);

  const filteredAndSorted = useMemo(() => {
    let result = symbolData.filter((s) => s.symbol.toLowerCase().includes(pairSearch.toLowerCase()));
    if (showFavoritesOnly) result = result.filter((s) => favorites.has(s.symbol));
    return [...result].sort((a, b) => {
      const dir = sortDirection === "asc" ? 1 : -1;
      if (sortColumn === "pair") return a.symbol.localeCompare(b.symbol) * dir;
      if (sortColumn === "spread") return ((a.spread ?? -999) - (b.spread ?? -999)) * dir;
      return ((a.exchangePrices[sortColumn]?.bid ?? 0) - (b.exchangePrices[sortColumn]?.bid ?? 0)) * dir;
    });
  }, [symbolData, pairSearch, showFavoritesOnly, favorites, sortColumn, sortDirection]);

  const sortIndicator = (col: string) => {
    if (sortColumn !== col) return null;
    return <span style={{ marginLeft: 2, color: "#6366f1" }}>{sortDirection === "asc" ? "↑" : "↓"}</span>;
  };

  const totalCols = 4 + visibleExchanges.length;

  // ---- Empty state ----
  if (prices.size === 0) {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "80vh", gap: 16, color: "var(--muted-foreground)" }}>
        <span style={{ fontSize: 48 }}>📡</span>
        <p style={{ fontSize: 18, fontWeight: 700, color: "var(--foreground)" }}>Waiting for price data…</p>
        <p style={{ fontSize: 14 }}>
          Connect the Rust backend on{" "}
          <code style={{ background: "var(--secondary)", padding: "2px 8px", borderRadius: 4, fontSize: 12, fontFamily: "monospace" }}>ws://localhost:8080/ws</code>
        </p>
        <Badge variant="secondary" className="capitalize">{status}</Badge>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", height: "calc(100vh - 3.5rem)" }}>
      {/* Sidebar */}
      <ExchangeSidebar
        allExchanges={allExchanges}
        selectedExchanges={selectedExchanges}
        onToggle={handleToggle}
        onSelectAll={handleSelectAll}
        onDeselectAll={handleDeselectAll}
        favorites={favorites}
        showFavoritesOnly={showFavoritesOnly}
        onShowFavoritesToggle={() => setShowFavoritesOnly(!showFavoritesOnly)}
      />

      {/* Main */}
      <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* Toolbar — matches original header */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "12px 24px",
            background: "var(--card)",
            borderBottom: "1px solid var(--border)",
          }}
        >
          <input
            type="text"
            value={pairSearch}
            onChange={(e) => setPairSearch(e.target.value)}
            placeholder="Filter pairs…"
            style={{
              width: 200,
              padding: "6px 12px",
              background: "var(--card)",
              border: "1px solid var(--border)",
              borderRadius: "var(--radius)",
              color: "var(--foreground)",
              fontSize: 13,
            }}
          />
          <span style={{ fontSize: 12, color: "var(--muted-foreground)" }}>
            {filteredAndSorted.length} pairs · {visibleExchanges.length} exchanges
          </span>
          {pinnedExchange && (
            <span style={{
              display: "inline-flex",
              alignItems: "center",
              padding: "4px 12px",
              fontSize: 12,
              fontWeight: 500,
              borderRadius: 9999,
              background: "rgba(245, 158, 11, 0.2)",
              color: "#f59e0b",
            }}>
              📌 {pinnedExchange}
            </span>
          )}
        </div>

        {/* Table */}
        <div style={{ flex: 1, overflow: "auto" }}>
          <table style={{ width: "100%", tableLayout: "fixed", borderCollapse: "separate", borderSpacing: 0 }}>
            <thead style={{ position: "sticky", top: 0, zIndex: 10, background: "transparent" }}>
              <tr>
                {/* Fav th */}
                <th style={{ width: 40, padding: "16px 6px 24px 12px", fontSize: 11, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.12em", color: "var(--muted-foreground)", textAlign: "center", borderBottom: "none" }}>
                  ★
                </th>
                {/* Pair th */}
                <th
                  onClick={() => handleSort("pair")}
                  style={{ width: 90, padding: "16px 12px 24px 12px", fontSize: 11, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.12em", color: "var(--muted-foreground)", textAlign: "left", cursor: "pointer", borderBottom: "none", whiteSpace: "nowrap" }}
                >
                  Pair {sortIndicator("pair")}
                </th>
                {/* Spread th */}
                <th
                  onClick={() => handleSort("spread")}
                  style={{ width: 80, padding: "16px 12px 24px 12px", fontSize: 11, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.12em", color: "var(--muted-foreground)", textAlign: "center", cursor: "pointer", borderBottom: "none", whiteSpace: "nowrap" }}
                >
                  Spread {sortIndicator("spread")}
                </th>
                {/* Strategy th */}
                <th style={{ width: 100, padding: "16px 12px 24px 12px", fontSize: 11, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.12em", color: "var(--muted-foreground)", textAlign: "center", borderBottom: "none" }}>
                  Strategy
                </th>
                {/* Exchange ths */}
                {visibleExchanges.map((exchange) => {
                  const isPinned = pinnedExchange === exchange;
                  return (
                    <th
                      key={exchange}
                      className="group"
                      style={{ width: 120, padding: "16px 12px 24px 12px", fontSize: 11, fontWeight: 500, textTransform: "uppercase", letterSpacing: "0.12em", color: "var(--muted-foreground)", textAlign: "center", borderBottom: "none", whiteSpace: "nowrap" }}
                    >
                      <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 4 }}>
                        <button
                          onClick={(e) => { e.stopPropagation(); handlePinExchange(exchange); }}
                          title={isPinned ? "Unpin" : "Pin exchange"}
                          className={cn(
                            "transition-all duration-150",
                            isPinned
                              ? "opacity-100"
                              : "opacity-0 group-hover:opacity-50 hover:!opacity-85"
                          )}
                          style={{
                            background: "none",
                            border: "none",
                            cursor: "pointer",
                            fontSize: 12,
                            lineHeight: 1,
                            padding: "2px 3px",
                            filter: isPinned ? "drop-shadow(0 0 4px rgba(251, 191, 36, 0.7))" : undefined,
                            transform: isPinned ? "scale(1.15)" : undefined,
                          }}
                        >
                          📌
                        </button>
                        <span
                          onClick={() => handleSort(exchange)}
                          style={{ cursor: "pointer" }}
                        >
                          {exchange} {sortIndicator(exchange)}
                        </span>
                      </div>
                    </th>
                  );
                })}
              </tr>
            </thead>

            <tbody>
              {filteredAndSorted.map((row) => {
                const isExpanded = expandedRow === row.symbol;
                const isFav = favorites.has(row.symbol);

                return (
                  <>
                    <tr
                      key={row.symbol}
                      className="group"
                      style={{
                        transition: "all 0.2s ease",
                        borderBottom: "1px solid rgba(255, 255, 255, 0.08)",
                        background: isExpanded ? "rgba(59, 130, 246, 0.1)" : undefined,
                        borderLeft: isExpanded ? "2px solid #3b82f6" : "2px solid transparent",
                      }}
                      onMouseEnter={(e) => {
                        if (!isExpanded) {
                          e.currentTarget.style.background = "rgba(99, 102, 241, 0.1)";
                          e.currentTarget.style.borderBottomColor = "rgba(99, 102, 241, 0.3)";
                        }
                      }}
                      onMouseLeave={(e) => {
                        if (!isExpanded) {
                          e.currentTarget.style.background = "";
                          e.currentTarget.style.borderBottomColor = "rgba(255, 255, 255, 0.08)";
                        }
                      }}
                    >
                      {/* ★ */}
                      <td style={{ padding: "20px 6px", textAlign: "center", borderBottom: "none" }}>
                        <button
                          onClick={() => handleFavoriteToggle(row.symbol)}
                          style={{
                            background: "none",
                            border: "none",
                            color: "#fbbf24",
                            cursor: "pointer",
                            fontSize: 14,
                            padding: 0,
                            opacity: isFav ? 1 : 0.4,
                            transition: "opacity 0.2s",
                            lineHeight: 1,
                          }}
                        >
                          ★
                        </button>
                      </td>

                      {/* PAIR */}
                      <td
                        style={{ padding: "20px 12px", textAlign: "left", cursor: "pointer", borderBottom: "none" }}
                        onClick={() => setExpandedRow(isExpanded ? null : row.symbol)}
                      >
                        <span style={{
                          fontWeight: 700,
                          fontSize: 15,
                          color: "#ffffff",
                          letterSpacing: "0.02em",
                        }}>
                          {row.symbol}
                        </span>
                      </td>

                      {/* SPREAD */}
                      <td style={{
                        padding: "20px 12px",
                        textAlign: "center",
                        fontFamily: "'JetBrains Mono', 'Monaco', 'Consolas', monospace",
                        fontSize: row.spread !== null && row.spread >= 0.5 ? 15 : 14,
                        fontWeight: row.spread !== null && row.spread >= 0.5 ? 700 : 500,
                        color: row.spread !== null && row.spread >= 0.5 ? "#10b981" : "var(--muted-foreground)",
                        borderBottom: "none",
                      }}>
                        {row.spread !== null ? `${row.spread.toFixed(4)}%` : "—"}
                      </td>

                      {/* STRATEGY — vertical layout matching original */}
                      <td style={{ padding: "20px 12px", textAlign: "center", borderBottom: "none" }}>
                        {row.longExchange && row.shortExchange && (
                          <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 4, fontSize: 9 }}>
                            <div>
                              <span style={{ color: "var(--muted-foreground)", textTransform: "uppercase", fontWeight: 500, marginRight: 4 }}>LONG</span>
                              <span style={{ background: "rgba(255,255,255,0.1)", padding: "2px 6px", borderRadius: 3, fontWeight: 600, color: "var(--foreground)" }}>
                                {row.longExchange}
                              </span>
                            </div>
                            <div>
                              <span style={{ color: "var(--muted-foreground)", textTransform: "uppercase", fontWeight: 500, marginRight: 4 }}>SHORT</span>
                              <span style={{ background: "rgba(255,255,255,0.1)", padding: "2px 6px", borderRadius: 3, fontWeight: 600, color: "var(--foreground)" }}>
                                {row.shortExchange}
                              </span>
                            </div>
                          </div>
                        )}
                      </td>

                      {/* Exchange prices */}
                      {visibleExchanges.map((exchange) => {
                        const p = row.exchangePrices[exchange];
                        if (!p) {
                          return (
                            <td key={exchange} style={{ padding: "20px 12px", textAlign: "center", color: "var(--muted-foreground)", opacity: 0.3, borderBottom: "none" }}>
                              —
                            </td>
                          );
                        }

                        const isBestBid = exchange === row.bestBidExchange;
                        const isBestAsk = exchange === row.bestAskExchange;
                        const isStrategyExchange = isBestBid || isBestAsk;

                        return (
                          <td key={exchange} style={{ padding: "20px 12px", borderBottom: "none" }}>
                            <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 6, fontFamily: "'JetBrains Mono', 'Monaco', 'Consolas', monospace" }}>
                              <span style={{
                                fontSize: isBestBid ? 14 : (isStrategyExchange ? 12 : 10),
                                fontWeight: isBestBid ? 700 : 400,
                                color: isBestBid ? "#ef4444" : (isStrategyExchange ? "#10b981" : "#64748b"),
                                textShadow: isBestBid ? "0 0 8px rgba(239, 68, 68, 0.5)" : undefined,
                              }}>
                                {formatPrice(p.bid)}
                              </span>
                              <span style={{
                                fontSize: isBestAsk ? 14 : (isStrategyExchange ? 12 : 10),
                                fontWeight: isBestAsk ? 700 : 400,
                                color: isBestAsk ? "#22c55e" : (isStrategyExchange ? "#ef4444" : "#64748b"),
                                textShadow: isBestAsk ? "0 0 8px rgba(34, 197, 94, 0.5)" : undefined,
                              }}>
                                {formatPrice(p.ask)}
                              </span>
                            </div>
                          </td>
                        );
                      })}
                    </tr>

                    {/* Chart expansion */}
                    {isExpanded && (
                      <SpreadChartPanel
                        key={`chart-${row.symbol}`}
                        symbol={row.symbol}
                        longExchange={row.longExchange}
                        shortExchange={row.shortExchange}
                        spread={row.spread}
                        onClose={() => setExpandedRow(null)}
                        colSpan={totalCols}
                      />
                    )}
                  </>
                );
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}
