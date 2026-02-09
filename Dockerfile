# ============================================================================
# Multi-stage Dockerfile for HFT Arbitrage Bot
# Stage 1: Build the Rust binary (heavy, discarded after build)
# Stage 2: Run with minimal image (small, fast startup)
# ============================================================================

# --- Stage 1: Build ---
FROM rust:1.84-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy Cargo manifest (and lock if present) for dependency caching
COPY bot/Cargo.toml ./
COPY bot/Cargo.lock* ./

# Create dummy main.rs + bins to build dependencies
RUN mkdir -p src/bin && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > src/bin/hft_bot.rs
RUN cargo build --release 2>/dev/null || true

# Now copy real source code + config
COPY bot/src ./src
COPY bot/config.yaml ./config.yaml

# Touch main to force rebuild of our code (not deps)
RUN touch src/main.rs

# Build the actual binary
RUN cargo build --release --bin hft_bot

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
