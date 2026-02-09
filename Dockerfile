# ============================================================================
# Multi-stage Dockerfile for HFT Arbitrage Bot
# Stage 1: Build the Rust binary (heavy, discarded after build)
# Stage 2: Run with minimal image (small, fast startup)
# ============================================================================

# --- Stage 1: Build ---
FROM rust:1.93-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy entire bot directory (source, benches, bins, config)
COPY bot/ ./

# Build release binary
RUN cargo build --release

# --- Stage 2: Runtime ---
FROM debian:bookworm-slim

# Install runtime dependencies (OpenSSL + CA certs for HTTPS)
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the built binary
COPY --from=builder /app/target/release/hft_bot /app/hft_bot

# Copy config
COPY bot/config.yaml /app/config.yaml

# Use JSON logs (no TUI in container)
ENV LOG_FORMAT=json
ENV RUST_LOG=hft_bot=info

# Run the bot
CMD ["./hft_bot"]
