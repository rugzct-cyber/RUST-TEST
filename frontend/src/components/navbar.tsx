"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useWS } from "@/components/providers";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

// =============================================================================
// Navigation items
// =============================================================================

const NAV_ITEMS = [
    { href: "/", label: "Dashboard" },
    { href: "/metrics", label: "Metrics" },
    { href: "/positions", label: "Positions" },
] as const;

// =============================================================================
// Navbar
// =============================================================================

export function Navbar() {
    const pathname = usePathname();
    const { status } = useWS();

    const statusColor: Record<string, string> = {
        connected: "bg-emerald-500",
        connecting: "bg-amber-500 animate-pulse",
        disconnected: "bg-red-500",
    };

    return (
        <header className="sticky top-0 z-50 border-b border-border/40 bg-background/80 backdrop-blur-xl">
            <div className="mx-auto flex h-14 max-w-screen-2xl items-center gap-6 px-6">
                {/* Logo */}
                <Link
                    href="/"
                    className="flex items-center gap-2 font-semibold tracking-tight"
                >
                    <span className="text-lg">⚡</span>
                    <span className="bg-gradient-to-r from-emerald-400 to-cyan-400 bg-clip-text text-transparent">
                        Arbi v5
                    </span>
                </Link>

                {/* Nav links */}
                <nav className="flex items-center gap-1">
                    {NAV_ITEMS.map((item) => (
                        <Link
                            key={item.href}
                            href={item.href}
                            className={cn(
                                "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                                pathname === item.href
                                    ? "bg-accent text-accent-foreground"
                                    : "text-muted-foreground hover:bg-accent/50 hover:text-foreground"
                            )}
                        >
                            {item.label}
                        </Link>
                    ))}
                </nav>

                {/* Spacer */}
                <div className="flex-1" />

                {/* Connection status */}
                <div className="flex items-center gap-2">
                    <span
                        className={cn("h-2 w-2 rounded-full", statusColor[status])}
                    />
                    <Badge variant={status === "connected" ? "default" : "secondary"}>
                        {status === "connected"
                            ? "Live"
                            : status === "connecting"
                                ? "Connecting…"
                                : "Offline"}
                    </Badge>
                </div>
            </div>
        </header>
    );
}
