"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useWS } from "@/components/providers";
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
// Navbar — matches original arbi-v5 Header style
// =============================================================================

export function Navbar() {
    const pathname = usePathname();
    const { status } = useWS();

    const statusColor = status === "connected"
        ? "#10b981"
        : status === "connecting"
            ? "#f59e0b"
            : "#ef4444";

    const statusClass = status === "connecting" ? "status-connecting" : "";

    return (
        <header
            style={{
                position: "sticky",
                top: 0,
                zIndex: 50,
                background: "var(--card)",
                borderBottom: "1px solid var(--border)",
                padding: "0 24px",
            }}
        >
            <div style={{
                display: "flex",
                alignItems: "center",
                height: 56,
                gap: 24,
                maxWidth: 1600,
                margin: "0 auto",
            }}>
                {/* Logo */}
                <Link
                    href="/"
                    style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        textDecoration: "none",
                        fontSize: "1.25rem",
                        fontWeight: 700,
                    }}
                >
                    <span style={{ fontSize: "1.5rem" }}>⚡</span>
                    <span style={{ color: "var(--foreground)" }}>Arbi v5</span>
                </Link>

                {/* Nav links */}
                <nav style={{ display: "flex", gap: 24 }}>
                    {NAV_ITEMS.map((item) => (
                        <Link
                            key={item.href}
                            href={item.href}
                            style={{
                                textDecoration: "none",
                                fontSize: "0.875rem",
                                fontWeight: pathname === item.href ? 500 : 400,
                                color: pathname === item.href ? "var(--foreground)" : "var(--muted-foreground)",
                            }}
                        >
                            {item.label}
                        </Link>
                    ))}
                </nav>

                {/* Spacer */}
                <div style={{ flex: 1 }} />

                {/* Connection status */}
                <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: "0.75rem", color: "var(--muted-foreground)" }}>
                    <span
                        className={statusClass}
                        style={{
                            width: 8,
                            height: 8,
                            borderRadius: "50%",
                            display: "inline-block",
                            background: statusColor,
                            boxShadow: status === "connected" ? `0 0 8px ${statusColor}` : undefined,
                        }}
                    />
                    <span>
                        {status === "connected" ? "Live" : status === "connecting" ? "Connecting…" : "Offline"}
                    </span>
                </div>
            </div>
        </header>
    );
}
