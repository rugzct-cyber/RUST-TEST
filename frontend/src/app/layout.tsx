import type { Metadata } from "next";
import { Inter, JetBrains_Mono } from "next/font/google";
import { ClientShell } from "@/components/client-shell";
import "./globals.css";

const inter = Inter({
  variable: "--font-sans",
  subsets: ["latin"],
});

const jetbrainsMono = JetBrains_Mono({
  variable: "--font-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "Arbi v5 | Real-Time Crypto Arbitrage Dashboard",
  description:
    "High-performance crypto arbitrage detection across multiple exchanges",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <body
        className={`${inter.variable} ${jetbrainsMono.variable} font-sans antialiased`}
      >
        <ClientShell>{children}</ClientShell>
      </body>
    </html>
  );
}
