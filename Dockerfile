# Build stage
FROM rust:1.91.1 AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy dependency manifests
COPY Cargo.toml Cargo.lock* ./

# Copy source code and assets
COPY src ./src
COPY cer.pem key.pem http-post.html ./

# Build the application in release mode
RUN cargo build --release

# Runtime stage
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /build/target/release/str0m-docker /app/str0m-docker

# Expose HTTPS port (web server)
EXPOSE 3000

# WebRTC uses dynamic UDP ports - this range should be mapped with -p flag
# We'll use a single ephemeral port per connection, assigned dynamically
EXPOSE 10000/udp

# Environment variables for Docker bridge mode
# PUBLIC_IP: The public/host IP that clients will connect to (REQUIRED in Docker)
# BIND_IP: The IP to bind the UDP socket to (default: 0.0.0.0)
ENV BIND_IP=0.0.0.0

CMD ["/app/str0m-docker"]
