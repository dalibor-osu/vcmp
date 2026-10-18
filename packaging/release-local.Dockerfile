# The build environment of packaging/release-local.sh: Ubuntu 22.04, like the Release
# workflow's Linux runner, with the libraries VCMP builds against on Linux, MinGW-w64
# for the Windows build, and Rust with both targets.
FROM ubuntu:22.04

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config zip \
        libfontconfig1-dev libxkbcommon-dev libasound2-dev \
        gcc-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*

# Pinned, so that rebuilding the image never changes the compiler unnoticed.
ARG RUST_VERSION=1.99.1
ENV RUSTUP_HOME=/usr/local/rustup PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | CARGO_HOME=/usr/local/cargo sh -s -- -y --no-modify-path --profile minimal \
            --default-toolchain "$RUST_VERSION" --target x86_64-pc-windows-gnu \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo
