"use client";

import { useState, useMemo } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

// =============================================================================
// ExchangeSidebar — filterable list of exchange checkboxes
// =============================================================================

interface ExchangeSidebarProps {
    /** All exchanges discovered from WebSocket data */
    allExchanges: string[];
    /** Currently selected (visible) exchanges */
    selectedExchanges: Set<string>;
    /** Toggle callback */
    onToggle: (exchange: string) => void;
    /** Select/deselect all */
    onSelectAll: () => void;
    onDeselectAll: () => void;
}

export function ExchangeSidebar({
    allExchanges,
    selectedExchanges,
    onToggle,
    onSelectAll,
    onDeselectAll,
}: ExchangeSidebarProps) {
    const [search, setSearch] = useState("");

    const filtered = useMemo(
        () =>
            allExchanges.filter((e) =>
                e.toLowerCase().includes(search.toLowerCase())
            ),
        [allExchanges, search]
    );

    const allSelected = selectedExchanges.size === allExchanges.length;

    return (
        <aside className="flex w-52 shrink-0 flex-col border-r border-border/30 bg-card/30 backdrop-blur-sm">
            {/* Search */}
            <div className="border-b border-border/20 p-3">
                <input
                    type="text"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    placeholder="Search pair…"
                    className="w-full rounded-md border border-border/30 bg-background/50 px-3 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:border-emerald-500/50 focus:outline-none focus:ring-1 focus:ring-emerald-500/30"
                />
            </div>

            {/* Select all / none */}
            <div className="flex items-center justify-between border-b border-border/20 px-3 py-2">
                <span className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                    Exchanges
                </span>
                <button
                    onClick={allSelected ? onDeselectAll : onSelectAll}
                    className="text-xs text-emerald-400 hover:text-emerald-300 transition-colors"
                >
                    {allSelected ? "None" : "All"}
                </button>
            </div>

            {/* Exchange list */}
            <ScrollArea className="flex-1">
                <div className="space-y-0.5 p-2">
                    {filtered.map((exchange) => {
                        const checked = selectedExchanges.has(exchange);
                        return (
                            <label
                                key={exchange}
                                className={cn(
                                    "flex cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-2 text-sm transition-colors",
                                    checked
                                        ? "bg-accent/50 text-foreground"
                                        : "text-muted-foreground hover:bg-accent/30 hover:text-foreground"
                                )}
                            >
                                <Checkbox
                                    checked={checked}
                                    onCheckedChange={() => onToggle(exchange)}
                                    className="h-3.5 w-3.5"
                                />
                                <span className="truncate font-medium uppercase text-xs tracking-wide">
                                    {exchange}
                                </span>
                            </label>
                        );
                    })}
                </div>
            </ScrollArea>

            {/* Footer count */}
            <div className="border-t border-border/20 px-3 py-2">
                <Badge variant="secondary" className="font-mono text-xs w-full justify-center">
                    {selectedExchanges.size} / {allExchanges.length}
                </Badge>
            </div>
        </aside>
    );
}
