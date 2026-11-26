# The binary is cross-compiled on the build machine's own platform: the Supervisor builds with BuildKit
# (docker buildx build --platform), which sets BUILDPLATFORM and TARGETPLATFORM, and Docker's xx
# helpers give cargo and the C compiler ring needs the target's linker and musl sysroot. On a
# Raspberry Pi both are aarch64 and the build is native. Tags and index digests pinned.
FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.10.0@sha256:40fe21f44bb8cddf1eb1226db190d0a984118f62ee02a91b77d99448d28264a6 AS xx

FROM --platform=$BUILDPLATFORM rust:1.98.1-alpine3.24@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS console

SHELL ["/bin/ash", "-o", "pipefail", "-c"]

# xx installs as a whole filesystem, the way its README gives it; it is a build stage only.
# hadolint ignore=DL3067
COPY --from=xx / /
# hadolint ignore=DL3018
RUN apk add --no-cache clang lld
ARG TARGETPLATFORM
# hadolint ignore=DL3018
RUN xx-apk add --no-cache gcc musl-dev
WORKDIR /src
COPY console/Cargo.toml console/Cargo.lock ./
COPY console/src ./src
RUN xx-cargo build --release --locked --target-dir /target \
 && bin="/target/$(xx-cargo --print-target-triple)/release/ai-console" \
 && xx-verify --static "$bin" \
 && install -m 755 "$bin" /ai-console

# Home Assistant's base image is published for amd64 and arm64 under one tag; the build's
# platform picks the matching one.
FROM ghcr.io/home-assistant/base:3.24

SHELL ["/bin/ash", "-o", "pipefail", "-c"]

ARG BUILD_ARCH=amd64
COPY --from=console /ai-console /usr/bin/ai-console
RUN { /usr/bin/ai-console 2>&1 || true; } | grep -q "ai-console serve"

# Options and the feed store live in /data.
CMD ["/usr/bin/ai-console", "serve", "--port", "8096", "--options", "/data/options.json", "--data", "/data"]

HEALTHCHECK --interval=60s --timeout=10s --start-period=30s --retries=3 \
    CMD ["/bin/sh", "-c", "curl -fsS -o /dev/null http://127.0.0.1:8096/healthz || exit 1"]

ARG BUILD_VERSION
LABEL \
    io.hass.name="AI Console" \
    io.hass.description="AI usage, plan limits, routes and providers on one page" \
    io.hass.arch="${BUILD_ARCH}" \
    io.hass.type="app" \
    io.hass.version="${BUILD_VERSION}" \
    maintainer="Lewis Heidrick" \
    org.opencontainers.image.title="AI Console" \
    org.opencontainers.image.description="AI usage, plan limits, routes and providers on one page" \
    org.opencontainers.image.authors="Lewis Heidrick" \
    org.opencontainers.image.licenses="MIT" \
    org.opencontainers.image.url="https://github.com/heidrickla/ha-ai-console" \
    org.opencontainers.image.source="https://github.com/heidrickla/ha-ai-console" \
    org.opencontainers.image.documentation="https://github.com/heidrickla/ha-ai-console/blob/main/ai_console/DOCS.md" \
    org.opencontainers.image.version="${BUILD_VERSION}"
