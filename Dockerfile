# syntax=docker/dockerfile:1

ARG RUST_IMAGE=rust:1.99.1-slim-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS build
WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --locked --release --bin traefik-network-controller

FROM ${RUNTIME_IMAGE} AS runtime
LABEL org.opencontainers.image.title="Traefik Network Controller" \
      org.opencontainers.image.description="Reconciles explicitly managed Docker application networks for one Traefik container" \
      org.opencontainers.image.licenses="MIT"

COPY --from=build /build/target/release/traefik-network-controller /usr/local/bin/traefik-network-controller
COPY LICENSE THIRD-PARTY-NOTICES.txt /usr/share/licenses/traefik-network-controller/
COPY third_party/rust/ /usr/share/licenses/traefik-network-controller/rust/
RUN mkdir -p /run/tnc

# The default root-owned Docker socket requires UID 0 in this deployment.
# Compose provides the writable state volume and restricts the remaining runtime.
USER 0:0
WORKDIR /
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/traefik-network-controller", "healthcheck"]

ENTRYPOINT ["/usr/local/bin/traefik-network-controller"]
CMD ["run"]
