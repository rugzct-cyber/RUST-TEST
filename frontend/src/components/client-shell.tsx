"use client";

import { usePathname } from "next/navigation";
import { Navbar } from "@/components/navbar";
import { WebSocketProvider } from "@/components/providers";

/**
 * Client layout wrapper — provides WebSocket context + Navbar
 * to all pages. Separated from root layout (server component)
 * because providers require "use client".
 */
export function ClientShell({ children }: { children: React.ReactNode }) {
    const pathname = usePathname();
    const isFullBleed = pathname === "/" || pathname === "/positions";

    return (
        <WebSocketProvider>
            <div className="relative flex min-h-screen flex-col bg-gradient-to-br from-[#0a0a1a] via-[#0f0f23] to-[#1a1a2e]">
                <Navbar />
                {isFullBleed ? (
                    <main className="flex-1">{children}</main>
                ) : (
                    <main className="flex-1 px-4 py-6 sm:px-6 lg:px-8">
                        <div className="mx-auto max-w-screen-2xl">{children}</div>
                    </main>
                )}
            </div>
        </WebSocketProvider>
    );
}
