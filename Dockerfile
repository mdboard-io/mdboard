# syntax=docker/dockerfile:1.7
FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY VERSION BUILD BUILD_DATE TAG ./
COPY sbin ./sbin
ARG MDBOARD_COMMIT=unknown
ENV MDBOARD_COMMIT=${MDBOARD_COMMIT}
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --locked --release -p mdboard-cli \
    && mkdir -p /out \
    && cp target/release/mdboard /out/mdboard

FROM debian:bookworm-slim
ARG VERSION=dev
LABEL org.opencontainers.image.title="mdboard"
LABEL org.opencontainers.image.description="Markdown-native, Git-backed board engine"
LABEL org.opencontainers.image.source="https://github.com/mdboard-io/mdboard"
LABEL org.opencontainers.image.licenses="AGPL-3.0-only"
LABEL org.opencontainers.image.version="${VERSION}"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 --shell /usr/sbin/nologin mdboard
COPY --from=builder /out/mdboard /usr/local/bin/mdboard
COPY LICENSE NOTICE /usr/share/doc/mdboard/
USER mdboard
WORKDIR /workspace
ENTRYPOINT ["tini", "--", "mdboard"]
CMD ["--help"]
