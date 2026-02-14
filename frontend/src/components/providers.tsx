"use client";

import { createContext, useContext, type ReactNode } from "react";
import { useWebSocket, type ConnectionStatus } from "@/hooks/useWebSocket";
import type { PriceData, ArbitrageOpportunity } from "@/lib/types";

// =============================================================================
// Context shape
// =============================================================================

interface WebSocketContextValue {
    status: ConnectionStatus;
    /** Map keyed by "exchange:symbol" → latest PriceData */
    prices: Map<string, PriceData>;
    /** Recent opportunities, newest first (ring-buffered) */
    opportunities: ArbitrageOpportunity[];
}

const WebSocketContext = createContext<WebSocketContextValue | null>(null);

// =============================================================================
// Provider — wraps the entire app, single WS connection
// =============================================================================

export function WebSocketProvider({ children }: { children: ReactNode }) {
    const ws = useWebSocket();

    return (
        <WebSocketContext.Provider value={ws}>{children}</WebSocketContext.Provider>
    );
}

// =============================================================================
// Consumer hook — type-safe access
// =============================================================================

export function useWS(): WebSocketContextValue {
    const ctx = useContext(WebSocketContext);
    if (!ctx) {
        throw new Error("useWS must be used within <WebSocketProvider>");
    }
    return ctx;
}
