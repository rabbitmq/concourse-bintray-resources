FROM rust:1.31.1-slim

WORKDIR /bintray-resources

COPY . /bintray-resources

RUN cargo build && \
    mkdir -p /opt/resource && \
    cp target/debug/bintray-package /opt/resource/

COPY ./scripts/* /opt/resource/
