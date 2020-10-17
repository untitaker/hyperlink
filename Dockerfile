# Note: This dockerfile is specifically designed to be run in GitHub actions.
# It may be unsuitable for most other purposes.

FROM rust:slim-stretch AS hyperlink-build

WORKDIR /work

# Build only dependencies to speed up subsequent builds
COPY Cargo.lock Cargo.toml ./
RUN mkdir -p src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked

# Build the actual app
COPY src ./src/
RUN cargo build --release --locked

FROM debian:stretch-slim
COPY --from=hyperlink-build /work/target/release/hyperlink /usr/bin/hyperlink
RUN chmod +x /usr/bin/hyperlink
