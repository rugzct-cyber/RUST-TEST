// =============================================================================
// Constants — Single source of truth for exchanges, symbols, and UI config
// =============================================================================

export const EXCHANGES = [
    "vest", "paradex", "lighter",
    "hyperliquid", "grvt", "reya",
    "hotstuff", "pacifica", "extended",
    "nado", "nord", "ethereal",
] as const;
export type Exchange = (typeof EXCHANGES)[number];

export const SYMBOLS = ["BTC", "ETH", "SOL"] as const;
export type Symbol = (typeof SYMBOLS)[number];

/** Display labels for exchanges */
export const EXCHANGE_LABELS: Record<Exchange, string> = {
    vest: "Vest",
    paradex: "Paradex",
    lighter: "Lighter",
    hyperliquid: "Hyperliquid",
    grvt: "GRVT",
    reya: "Reya",
    hotstuff: "HotStuff",
    pacifica: "Pacifica",
    extended: "Extended",
    nado: "Nado",
    nord: "Nord",
    ethereal: "Ethereal",
};

/** Accent colors per exchange */
export const EXCHANGE_COLORS: Record<Exchange, string> = {
    vest: "#3b82f6",        // blue
    paradex: "#a855f7",     // purple
    lighter: "#22c55e",     // green
    hyperliquid: "#06b6d4", // cyan
    grvt: "#f97316",        // orange
    reya: "#ec4899",        // pink
    hotstuff: "#ef4444",    // red
    pacifica: "#14b8a6",    // teal
    extended: "#8b5cf6",    // violet
    nado: "#eab308",        // yellow
    nord: "#6366f1",        // indigo
    ethereal: "#d946ef",    // fuchsia
};

/** Chart colors for spread lines */
export const CHART_COLORS = [
    "#4ecdc4", // teal
    "#ff6b6b", // coral
    "#ffd93d", // yellow
    "#a855f7", // purple
    "#3b82f6", // blue
    "#22c55e", // green
] as const;

/** WebSocket backend URL */
export const WS_URL =
    process.env.NEXT_PUBLIC_WS_URL || "ws://localhost:8080/ws";

/** REST API base URL */
export const API_URL =
    process.env.NEXT_PUBLIC_API_URL || "http://localhost:8080";

/** Maximum data points to keep in ring buffers */
export const MAX_CHART_POINTS = 300;

/** Price staleness threshold in ms */
export const STALE_THRESHOLD_MS = 5_000;
export const WARN_THRESHOLD_MS = 2_000;
