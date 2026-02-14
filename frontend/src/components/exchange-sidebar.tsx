"use client";

import { useState, useMemo } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

// =============================================================================
// ExchangeSidebar — matches original arbi-v5 Sidebar.module.css
// =============================================================================

interface ExchangeSidebarProps {
    allExchanges: string[];
    selectedExchanges: Set<string>;
    onToggle: (exchange: string) => void;
    onSelectAll: () => void;
    onDeselectAll: () => void;
    favorites?: Set<string>;
    showFavoritesOnly?: boolean;
    onShowFavoritesToggle?: () => void;
}

export function ExchangeSidebar({
    allExchanges,
    selectedExchanges,
    onToggle,
    onSelectAll,
    onDeselectAll,
    favorites,
    showFavoritesOnly = false,
    onShowFavoritesToggle,
}: ExchangeSidebarProps) {
    const [search, setSearch] = useState("");
    const [isCollapsed, setIsCollapsed] = useState(false);

    const filtered = useMemo(
        () => allExchanges.filter((e) => e.toLowerCase().includes(search.toLowerCase())),
        [allExchanges, search]
    );

    const allSelected = selectedExchanges.size === allExchanges.length;
    const favCount = favorites?.size ?? 0;

    if (isCollapsed) {
        return (
            <aside
                style={{
                    width: 40,
                    flexShrink: 0,
                    display: "flex",
                    flexDirection: "column",
                    alignItems: "center",
                    background: "var(--card)",
                    borderRight: "1px solid var(--border)",
                    paddingTop: 16,
                    gap: 8,
                }}
            >
                <button
                    onClick={() => setIsCollapsed(false)}
                    title="Expand sidebar"
                    style={{
                        background: "none",
                        border: "none",
                        color: "var(--muted-foreground)",
                        cursor: "pointer",
                        fontSize: 14,
                    }}
                >
                    ▶
                </button>
                <span style={{
                    fontSize: 10,
                    padding: "2px 6px",
                    borderRadius: 9999,
                    background: "var(--secondary)",
                    color: "var(--secondary-foreground)",
                }}>
                    {selectedExchanges.size}
                </span>
            </aside>
        );
    }

    return (
        <aside
            style={{
                width: 220,
                flexShrink: 0,
                display: "flex",
                flexDirection: "column",
                background: "var(--card)",
                borderRight: "1px solid var(--border)",
            }}
        >
            {/* Header */}
            <div style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                padding: "12px 16px",
                borderBottom: "1px solid var(--border)",
            }}>
                <span style={{ fontSize: 11, fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.1em", color: "var(--muted-foreground)" }}>
                    Filters
                </span>
                <button
                    onClick={() => setIsCollapsed(true)}
                    title="Collapse sidebar"
                    style={{
                        background: "none",
                        border: "none",
                        color: "var(--muted-foreground)",
                        cursor: "pointer",
                        fontSize: 12,
                    }}
                >
                    ◀
                </button>
            </div>

            {/* Search */}
            <div style={{ padding: "12px 12px", borderBottom: "1px solid var(--border)" }}>
                <input
                    type="text"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="Search exchange…"
                    style={{
                        width: "100%",
                        padding: "6px 10px",
                        background: "var(--card)",
                        border: "1px solid var(--border)",
                        borderRadius: "var(--radius)",
                        color: "var(--foreground)",
                        fontSize: 13,
                    }}
                />
            </div>

            {/* Favorites filter */}
            {onShowFavoritesToggle && (
                <div style={{ padding: "8px 12px", borderBottom: "1px solid var(--border)" }}>
                    <button
                        onClick={onShowFavoritesToggle}
                        style={{
                            display: "flex",
                            width: "100%",
                            alignItems: "center",
                            gap: 8,
                            padding: "8px 12px",
                            borderRadius: "var(--radius)",
                            fontSize: 13,
                            fontWeight: 500,
                            background: showFavoritesOnly ? "rgba(99, 102, 241, 0.15)" : "transparent",
                            border: showFavoritesOnly ? "1px solid rgba(99, 102, 241, 0.3)" : "1px solid transparent",
                            color: showFavoritesOnly ? "#6366f1" : "var(--muted-foreground)",
                            cursor: "pointer",
                            transition: "all 0.2s ease",
                        }}
                    >
                        <span>★</span>
                        <span style={{ fontSize: 11, textTransform: "uppercase", letterSpacing: "0.05em" }}>Favorites</span>
                        {favCount > 0 && (
                            <span style={{
                                marginLeft: "auto",
                                fontSize: 10,
                                padding: "1px 6px",
                                borderRadius: 9999,
                                background: "var(--secondary)",
                                color: "var(--secondary-foreground)",
                            }}>
                                {favCount}
                            </span>
                        )}
                    </button>
                </div>
            )}

            {/* Exchanges header */}
            <div style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                padding: "8px 16px",
                borderBottom: "1px solid var(--border)",
            }}>
                <span style={{ fontSize: 11, fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.1em", color: "var(--muted-foreground)" }}>
                    Exchanges
                </span>
                <button
                    onClick={allSelected ? onDeselectAll : onSelectAll}
                    style={{
                        background: "none",
                        border: "none",
                        fontSize: 11,
                        fontWeight: 600,
                        color: "#6366f1",
                        cursor: "pointer",
                    }}
                >
                    {allSelected ? "None" : "All"}
                </button>
            </div>

            {/* Exchange list */}
            <ScrollArea className="flex-1">
                <div style={{ padding: 8 }}>
                    {filtered.map((exchange) => {
                        const checked = selectedExchanges.has(exchange);
                        return (
                            <label
                                key={exchange}
                                style={{
                                    display: "flex",
                                    alignItems: "center",
                                    gap: 10,
                                    padding: "10px 12px",
                                    borderRadius: "var(--radius)",
                                    fontSize: 13,
                                    cursor: "pointer",
                                    transition: "all 0.2s ease",
                                    background: checked ? "rgba(99, 102, 241, 0.1)" : "transparent",
                                    color: checked ? "var(--foreground)" : "var(--muted-foreground)",
                                }}
                            >
                                <Checkbox
                                    checked={checked}
                                    onCheckedChange={() => onToggle(exchange)}
                                    className="h-3.5 w-3.5"
                                />
                                <span style={{ fontWeight: 600, fontSize: 11, textTransform: "uppercase", letterSpacing: "0.05em" }}>
                                    {exchange}
                                </span>
                            </label>
                        );
                    })}
                </div>
            </ScrollArea>

            {/* Footer */}
            <div style={{
                padding: "8px 16px",
                borderTop: "1px solid var(--border)",
                textAlign: "center",
                fontSize: 12,
                color: "var(--muted-foreground)",
            }}>
                <span style={{ fontFamily: "monospace" }}>{selectedExchanges.size}</span> / <span style={{ fontFamily: "monospace" }}>{allExchanges.length}</span>
            </div>
        </aside>
    );
}
