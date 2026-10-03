# syntax=docker/dockerfile:1
# The web build of LifeAgent for Cloud Run (docs/HACKATHON.md §1): the React UI and
# lifeagent-server in one image, the server serving the UI from LIFEAGENT_STATIC_DIR.
# Built with Cloud Build: see deploy/README.md.

# ---- UI ---------------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS ui
WORKDIR /src/apps/web
# Manifest and lockfile first, so `npm ci` is cached until dependencies change.
COPY apps/web/package.json apps/web/package-lock.json ./
RUN npm ci
COPY apps/web ./
RUN npm run build

# ---- Server -----------------------------------------------------------------------------------
FROM rust:1-bookworm AS server
WORKDIR /src
# The whole workspace is needed for Cargo to resolve it, but only the server and its
# dependencies are compiled.
COPY . .
RUN cargo build --release --locked -p lifeagent-server

# ---- Runtime ----------------------------------------------------------------------------------
FROM debian:bookworm-slim
# CA roots for HTTPS to Google (Agent Platform, OAuth); tzdata so TZ=Asia/Tokyo resolves.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --no-create-home lifeagent
COPY --from=server /src/target/release/lifeagent-server /usr/local/bin/lifeagent-server
COPY --from=ui /src/apps/web/dist /app/ui
ENV LIFEAGENT_STATIC_DIR=/app/ui
USER lifeagent
# Cloud Run sets PORT; the server listens on it (default 8080).
EXPOSE 8080
CMD ["lifeagent-server"]
