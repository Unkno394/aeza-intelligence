FROM rust:1.94-slim-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM debian:bookworm-slim
COPY --from=build /app/target/release/aeza-market-simulator /usr/local/bin/market-simulator
COPY data /app/data
ENV MARKET_BIND=0.0.0.0:8091 \
    MARKET_DATA_PATH=/app/data/market-snapshot.json
EXPOSE 8091
ENTRYPOINT ["market-simulator"]
