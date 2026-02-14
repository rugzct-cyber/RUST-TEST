// =============================================================================
// TypeScript types matching the Rust backend's BroadcastEvent enum
// =============================================================================

/** Price update from a single exchange */
export interface PriceData {
    exchange: string;
    symbol: string;
    bid: number;
    ask: number;
    timestamp_ms: number;
}

/** Detected arbitrage opportunity */
export interface ArbitrageOpportunity {
    symbol: string;
    buy_exchange: string;
    sell_exchange: string;
    buy_price: number;
    sell_price: number;
    spread_percent: number;
    timestamp_ms: number;
    confirmations: number;
}

/**
 * Discriminated union matching Rust's BroadcastEvent.
 * Rust uses `#[serde(tag = "type", content = "data")]`, producing:
 *   { "type": "price",       "data": { ... } }
 *   { "type": "opportunity", "data": { ... } }
 */
export type BroadcastEvent =
    | { type: "price"; data: PriceData }
    | { type: "opportunity"; data: ArbitrageOpportunity }
    | { type: "exchange_status"; data: { exchange: string; connected: boolean } };

/** Aggregated price snapshot from REST /api/prices */
export interface AggregatedPrice {
    symbol: string;
    best_bid: number;
    best_ask: number;
    best_bid_exchange: string;
    best_ask_exchange: string;
    spread_percent: number;
    exchange_count: number;
    timestamp_ms: number;
}

// =============================================================================
// Position Management types
// =============================================================================

/** A tracked arbitrage position (long on one exchange, short on another) */
export interface Position {
    id: string;
    token: string;
    longExchange: string;
    shortExchange: string;
    entryPriceLong: number;
    entryPriceShort: number;
    tokenAmount: number;
    entrySpread: number;
    timestamp: number;
}

/** A single point in the exit spread time-series chart */
export interface SpreadPoint {
    time: string;
    timestamp: number;
    exitSpread: number;
    longBid: number;
    shortAsk: number;
}

/** Live exit spread data computed from current prices */
export interface ExitSpreadData {
    exitSpread: number;
    exitSpreadDollar: number;
    longBid: number;
    longAsk: number;
    shortBid: number;
    shortAsk: number;
    isInProfit: boolean;
    pnl: number;
}
