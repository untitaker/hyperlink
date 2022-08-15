FROM rust:1.63-alpine3.16 as builder

RUN apk add --no-cache musl-dev

RUN USER=root cargo new --bin /hyperlink
WORKDIR /hyperlink

# Just copy the Cargo.toml files and trigger
# a build so that we compile our dependencies only.
# This way we avoid layer cache invalidation
# if our dependencies haven't changed,
# resulting in faster builds.

COPY Cargo.toml .
COPY Cargo.lock .
RUN cargo build --release && rm -rf src/
RUN strip target/release/hyperlink

# Copy the source code and run the build again.
# This should only compile lychee itself as the
# dependencies were already built above.
COPY . ./
RUN rm ./target/release/deps/hyperlink* && cargo build --release

# Our production image starts here, which uses
# the files from the builder image above.
FROM alpine:3.16

COPY --from=builder /hyperlink/target/release/hyperlink /usr/local/bin/hyperlink

ENTRYPOINT ["hyperlink"]
