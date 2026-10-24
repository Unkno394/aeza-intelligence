FROM rust:1.94-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY migrations migrations
RUN cargo build --release --locked --bins

FROM debian:bookworm-slim
# search_interest calls curl by default (--transport curl).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/aeza_intelligence /usr/local/bin/aeza_intelligence
COPY --from=build /app/target/release/market_api /usr/local/bin/market_api
COPY --from=build /app/target/release/client_snapshot /usr/local/bin/client_snapshot
COPY --from=build /app/target/release/search_interest /usr/local/bin/search_interest
COPY --from=build /app/target/release/market_import /usr/local/bin/market_import
COPY --from=build /app/target/release/market_fx /usr/local/bin/market_fx
COPY --from=build /app/target/release/google_trends_import /usr/local/bin/google_trends_import
ENTRYPOINT ["aeza_intelligence"]
