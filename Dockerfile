FROM rust:1.99.1-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml openapi.yaml ./
COPY src ./src
COPY api ./api
RUN cargo build --release --locked --bin calendar-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 calendar \
    && useradd --uid 10001 --gid 10001 --no-create-home --shell /usr/sbin/nologin calendar
COPY --from=build /build/target/release/calendar-server /usr/local/bin/calendar-server
USER calendar
ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/calendar-server"]
