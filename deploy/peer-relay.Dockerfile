FROM rust:1.96-bookworm AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
RUN cargo build --release --locked -p cloud-peer-relay

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home relay
COPY --from=build /src/target/release/cloud-peer-relay /usr/local/bin/cloud-peer-relay
USER 10001
EXPOSE 8443/tcp 7842/udp
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 CMD curl --fail --silent http://127.0.0.1:8091/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/cloud-peer-relay"]
