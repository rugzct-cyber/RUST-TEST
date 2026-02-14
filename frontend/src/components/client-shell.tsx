"use client";

import { usePathname } from "next/navigation";
import { Navbar } from "@/components/navbar";
import { WebSocketProvider } from "@/components/providers";

export function ClientShell({ children }: { children: React.ReactNode }) {
    const pathname = usePathname();
    const isFullBleed = pathname === "/" || pathname === "/positions";

    return (
        <WebSocketProvider>
            <div style={{ position: "relative", display: "flex", flexDirection: "column", minHeight: "100vh", background: "var(--background)" }}>
                <Navbar />
                {isFullBleed ? (
                    <main style={{ flex: 1 }}>{children}</main>
                ) : (
                    <main style={{ flex: 1, padding: "24px 16px" }}>
                        <div style={{ maxWidth: 1400, margin: "0 auto" }}>{children}</div>
                    </main>
                )}
            </div>
        </WebSocketProvider>
    );
}
