FROM ekidd/rust-musl-builder AS builder

WORKDIR /bintray-resources

COPY . /bintray-resources

# Fix permissions on source code.
RUN sudo chown -R rust:rust /bintray-resources

RUN cargo build --release --target x86_64-unknown-linux-musl

FROM watawuwu/openssl:latest

RUN mkdir -p /opt/resource

COPY --from=builder /bintray-resources/target/x86_64-unknown-linux-musl/release/bintray-package /opt/resource/
COPY --from=builder /bintray-resources/assets/* /opt/resource/