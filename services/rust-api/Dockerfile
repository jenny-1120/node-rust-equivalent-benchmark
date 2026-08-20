FROM rust:1.81-alpine AS build
WORKDIR /app
RUN apk add --no-cache musl-dev
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM alpine:3.20
WORKDIR /app
RUN apk add --no-cache libgcc
COPY --from=build /app/target/release/phasea-rust-api /usr/local/bin/phasea-rust-api
EXPOSE 3002
CMD ["phasea-rust-api"]
