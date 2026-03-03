FROM rust:1-slim-bookworm

WORKDIR /app

# Install dependencies (ALSA and build tools)
RUN apt-get update && apt-get install -y \
    libasound2-dev \
    pkg-config \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace members
COPY . .

# Build the server crate
RUN cargo build --release -p server

# Create archive and recordings dir
RUN mkdir -p /app/archive /app/recordings

EXPOSE 8080

CMD ["./target/release/server"]
