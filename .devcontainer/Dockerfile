# decel development container: Rust stable, GPUI Linux build/runtime deps,
# cargo-deny, cargo-nextest, just, typst (+ cetz/fletcher diagram packages).
FROM mcr.microsoft.com/devcontainers/rust:1-trixie

ARG BINSTALL_VERSION=1.26.2
ARG JUST_VERSION=1.59.0
ARG CARGO_DENY_VERSION=0.20.2
ARG NEXTEST_VERSION=0.9.150
ARG TYPST_VERSION=0.15.1
ARG CETZ_VERSION=0.5.3
ARG FLETCHER_VERSION=0.5.8

# GPUI build deps (DESIGN §10.1) plus Mesa Vulkan drivers and fonts so the
# window can actually render when the host GPU and Wayland/X11 sockets are
# passed through.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential clang cmake pkg-config \
      libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
      libxcb1-dev libx11-xcb-dev libx11-dev \
      libfontconfig-dev libfreetype-dev libzstd-dev \
      libvulkan1 libvulkan-dev mesa-vulkan-drivers vulkan-tools \
      libgl1 libegl1 libasound2-dev \
      fonts-dejavu fonts-noto-core fonts-jetbrains-mono \
      xz-utils \
 && rm -rf /var/lib/apt/lists/*

# Typst (single static binary).
RUN curl -fsSL "https://github.com/typst/typst/releases/download/v${TYPST_VERSION}/typst-x86_64-unknown-linux-musl.tar.xz" \
      | tar -xJ -C /tmp \
 && install -m 0755 /tmp/typst-x86_64-unknown-linux-musl/typst /usr/local/bin/typst \
 && rm -rf /tmp/typst-x86_64-unknown-linux-musl

USER vscode

# Cargo tools as prebuilt binaries via cargo-binstall (pinned versions).
RUN curl -fsSL "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL_VERSION}/cargo-binstall-x86_64-unknown-linux-musl.tgz" \
      | tar -xz -C "${CARGO_HOME}/bin" \
 && cargo binstall -y --disable-telemetry \
      "just@${JUST_VERSION}" \
      "cargo-deny@${CARGO_DENY_VERSION}" \
      "cargo-nextest@${NEXTEST_VERSION}" \
 && rustup component add rustfmt clippy llvm-tools-preview

# Pre-fetch typst diagram packages into the user's package cache so slides
# build offline.
RUN printf '#import "@preview/cetz:%s"\n#import "@preview/fletcher:%s"\nok\n' \
      "${CETZ_VERSION}" "${FLETCHER_VERSION}" > /tmp/warm.typ \
 && typst compile /tmp/warm.typ /tmp/warm.pdf \
 && rm /tmp/warm.typ /tmp/warm.pdf
