FROM rust:1.94-slim-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY migrations migrations
RUN cargo build --release --locked

FROM debian:bookworm-slim
COPY --from=build /app/target/release/aeza_intelligence /usr/local/bin/aeza_intelligence
ENTRYPOINT ["aeza_intelligence"]
