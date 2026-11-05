# Build with the repository root as the context. No database is bundled.
FROM node:22.23.4-bookworm-slim AS admin
WORKDIR /src/web/admin
COPY web/admin/package.json web/admin/package-lock.json ./
RUN npm ci --ignore-scripts
COPY web/admin/ ./
RUN npm run build

FROM rust:1.96-bookworm AS backend
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY migrations/ ./migrations/
RUN cargo build --release --locked -p cloud-api

# Includes PostgreSQL 18 pg_dump/pg_restore for explicit offline backups. The
# database server is not started by this image.
FROM postgres:18-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends clamav-daemon && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home cloud
COPY --from=backend /src/target/release/cloud-api /usr/local/bin/cloud-api
COPY --from=admin /src/web/admin/dist /opt/cloud-admin
COPY deploy/start-cloud /usr/local/bin/start-cloud
COPY deploy/clamd.conf.example /opt/cloud-clamd.conf.example
USER 10001
ENV ADMIN_DIST_DIR=/opt/cloud-admin
ENTRYPOINT []
CMD ["bash", "/usr/local/bin/start-cloud"]
