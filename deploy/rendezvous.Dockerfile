# The rendezvous server as a container: what `relay remote via wss://…` points at.
#
#   docker build -f deploy/rendezvous.Dockerfile -t relay-rendezvous .
#   docker run -d --name relay-rendezvous -p 127.0.0.1:7430:7430 relay-rendezvous
#
# Put TLS in front (docs/MOBILE.md §2); the container itself speaks plain ws:// on 7430.
# Only the headless crates are built, so no GTK is needed.

FROM rust:1.94-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps/relay-native/Cargo.toml ./apps/relay-native/Cargo.toml
RUN mkdir -p apps/relay-native/src && echo 'fn main() {}' > apps/relay-native/src/main.rs
RUN cargo build --release --locked -p relay-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/relay /usr/local/bin/relay
USER 65534:65534
EXPOSE 7430
ENV RELAY_LOG=info
ENTRYPOINT ["relay", "remote", "rendezvous", "--bind", "0.0.0.0:7430"]
