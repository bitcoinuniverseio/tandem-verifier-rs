FROM rust:1.97.1-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --locked --release -p tandem-verifier

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 tandem
COPY --from=builder /src/target/release/tandem-verifier /usr/local/bin/tandem-verifier
USER 10001:10001
EXPOSE 8088
ENTRYPOINT ["/usr/local/bin/tandem-verifier"]

