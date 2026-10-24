FROM rust:1.94-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY migrations migrations
RUN cargo build --release --locked --bins

FROM debian:bookworm-slim
COPY --from=build /app/target/release/aeza_intelligence /usr/local/bin/aeza_intelligence
COPY --from=build /app/target/release/market_api /usr/local/bin/market_api
COPY --from=build /app/target/release/client_snapshot /usr/local/bin/client_snapshot
COPY --from=build /app/target/release/search_interest /usr/local/bin/search_interest
ENTRYPOINT ["aeza_intelligence"]
