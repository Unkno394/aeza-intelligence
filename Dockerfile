FROM rust:1.94-slim-bookworm AS build
WORKDIR /app
ENV CARGO_NET_RETRY=8 \
    CARGO_HTTP_TIMEOUT=120 \
    CARGO_HTTP_LOW_SPEED_LIMIT=1
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
RUN cargo fetch --locked
COPY migrations migrations
COPY docs/openapi-v1.json docs/openapi-v1.json
RUN cargo build --release --locked --offline --bins

FROM debian:bookworm-slim
# search_interest calls curl by default (--transport curl).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/aeza_intelligence /usr/local/bin/aeza_intelligence
COPY --from=build /app/target/release/market_api /usr/local/bin/market_api
COPY --from=build /app/target/release/client_snapshot /usr/local/bin/client_snapshot
COPY --from=build /app/target/release/demo_stream /usr/local/bin/demo_stream
COPY --from=build /app/target/release/search_interest /usr/local/bin/search_interest
COPY --from=build /app/target/release/market_import /usr/local/bin/market_import
COPY --from=build /app/target/release/market_fx /usr/local/bin/market_fx
COPY --from=build /app/target/release/google_trends_import /usr/local/bin/google_trends_import
ENTRYPOINT ["aeza_intelligence"]
