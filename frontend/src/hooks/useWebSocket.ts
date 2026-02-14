"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import type { PriceData, ArbitrageOpportunity, BroadcastEvent } from "@/lib/types";
import { WS_URL, MAX_CHART_POINTS } from "@/lib/constants";

// =============================================================================
// Connection state
// =============================================================================

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

// =============================================================================
// Hook: useWebSocket
// =============================================================================

/**
 * Central WebSocket hook that connects to the Rust backend.
 *
 * Design decisions:
 * - Uses native WebSocket (not Socket.IO) since the Rust backend sends raw JSON
 * - Accumulates prices in a ref, throttles React state updates to 250ms
 * - Ring-buffers opportunity history to prevent memory leaks
 * - Auto-reconnects with exponential backoff (1s → 2s → 4s → max 30s)
 */
export function useWebSocket() {
    const [status, setStatus] = useState<ConnectionStatus>("disconnected");
    const [prices, setPrices] = useState<Map<string, PriceData>>(new Map());
    const [opportunities, setOpportunities] = useState<ArbitrageOpportunity[]>([]);

    // Refs for accumulation (avoid re-renders on every tick)
    const priceBuffer = useRef<Map<string, PriceData>>(new Map());
    const oppBuffer = useRef<ArbitrageOpportunity[]>([]);
    const wsRef = useRef<WebSocket | null>(null);
    const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
    const flushTimer = useRef<ReturnType<typeof setInterval> | null>(null);
    const backoffMs = useRef(1000);

    // Flush buffered data to React state (throttled)
    const flush = useCallback(() => {
        if (priceBuffer.current.size > 0) {
            setPrices(new Map(priceBuffer.current));
        }
        if (oppBuffer.current.length > 0) {
            setOpportunities([...oppBuffer.current]);
        }
    }, []);

    // Connect to WebSocket
    const connect = useCallback(() => {
        if (wsRef.current?.readyState === WebSocket.OPEN) return;

        setStatus("connecting");

        try {
            const ws = new WebSocket(WS_URL);
            wsRef.current = ws;

            ws.onopen = () => {
                setStatus("connected");
                backoffMs.current = 1000; // Reset backoff on success
            };

            ws.onmessage = (event) => {
                try {
                    const msg: BroadcastEvent = JSON.parse(event.data);

                    if (msg.type === "price") {
                        const price = msg.data;
                        const key = `${price.exchange}:${price.symbol}`;
                        priceBuffer.current.set(key, price);
                    } else if (msg.type === "opportunity") {
                        const opp = msg.data;
                        oppBuffer.current = [opp, ...oppBuffer.current].slice(
                            0,
                            MAX_CHART_POINTS
                        );
                    }
                } catch {
                    // Ignore malformed messages
                }
            };

            ws.onclose = () => {
                setStatus("disconnected");
                wsRef.current = null;
                scheduleReconnect();
            };

            ws.onerror = () => {
                ws.close();
            };
        } catch {
            setStatus("disconnected");
            scheduleReconnect();
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // Exponential backoff reconnect
    const scheduleReconnect = useCallback(() => {
        if (reconnectTimer.current) return;

        reconnectTimer.current = setTimeout(() => {
            reconnectTimer.current = null;
            backoffMs.current = Math.min(backoffMs.current * 2, 30_000);
            connect();
        }, backoffMs.current);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [connect]);

    // Lifecycle
    useEffect(() => {
        connect();

        // Flush buffer to state every 250ms (high-frequency data throttle)
        flushTimer.current = setInterval(flush, 250);

        return () => {
            wsRef.current?.close();
            wsRef.current = null;
            if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
            if (flushTimer.current) clearInterval(flushTimer.current);
        };
    }, [connect, flush]);

    return { status, prices, opportunities };
}
