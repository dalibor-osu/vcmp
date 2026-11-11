ARG RUST_VERSION=1.99.1

FROM rust:${RUST_VERSION}-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo build --release --locked -p vcmp-server

FROM debian:trixie-slim
RUN useradd --system --uid 10001 --user-group --no-create-home vcmp \
    && mkdir -p /data /music \
    && chown vcmp:vcmp /data
COPY --from=build /src/target/release/vcmp-server /usr/local/bin/vcmp-server
ENV VCMP_SERVER_DATA=/data \
    VCMP_SERVER_MUSIC=/music \
    VCMP_SERVER_LISTEN=0.0.0.0:8268
USER vcmp
VOLUME ["/data"]
EXPOSE 8268
ENTRYPOINT ["vcmp-server"]
CMD ["serve"]
