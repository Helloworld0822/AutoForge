# syntax=docker/dockerfile:1

FROM docker.io/library/rust:1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml ./
COPY Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM docker.io/library/debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --shell /usr/sbin/nologin autoforge

WORKDIR /app
COPY --from=builder /build/target/release/autoforge /usr/local/bin/autoforge

ENV HOST=0.0.0.0
ENV PORT=8080
ENV RUST_LOG=info

USER autoforge
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/autoforge"]
CMD ["serve"]
