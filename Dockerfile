FROM lukemathwalker/cargo-chef:0.1.51-rust-1.66.0 as chef
WORKDIR app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release

# We do not need the Rust toolchain to run the binary!
FROM debian:latest AS runtime
WORKDIR app
RUN apt update -y && apt install -y ca-certificates
COPY --from=builder /app/target/release/power-watcher /usr/local/bin
ENTRYPOINT ["/usr/local/bin/power-watcher"]
