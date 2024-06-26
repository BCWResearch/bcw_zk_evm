FROM rustlang/rust:nightly-bullseye-slim as builder

RUN apt-get update && apt-get install -y libjemalloc2 libjemalloc-dev make libssl-dev pkg-config

RUN \
    mkdir -p ops/src     && touch ops/src/lib.rs && \
    mkdir -p common/src  && touch common/src/lib.rs && \
    mkdir -p rpc/src     && touch rpc/src/lib.rs && \
    mkdir -p prover/src  && touch prover/src/lib.rs && \
    mkdir -p leader/src && touch leader/src/lib.rs && \
    mkdir -p coordinator/src  && echo "fn main() {println!(\"coordinator main\");}" > coordinator/src/main.rs

COPY Cargo.toml .
RUN sed -i "2s/.*/members = [\"ops\", \"leader\", \"common\", \"rpc\", \"prover\", \"coordinator\"]/" Cargo.toml
COPY Cargo.lock .

COPY zero_bin/ops/Cargo.toml ./ops/Cargo.toml
COPY zero_bin/common/Cargo.toml ./common/Cargo.toml
COPY zero_bin/rpc/Cargo.toml ./rpc/Cargo.toml
COPY zero_bin/prover/Cargo.toml ./prover/Cargo.toml
COPY zero_bin/leader/Cargo.toml ./leader/Cargo.toml
COPY zero_bin/coordinator/Cargo.toml ./coordinator/Cargo.toml

COPY ./rust-toolchain.toml ./

RUN cargo build --verbose --release --bin coordinator

COPY zero_bin/coordinator ./coordinator
COPY zero_bin/ops ./ops
COPY zero_bin/common ./common
COPY zero_bin/rpc ./rpc
COPY zero_bin/prover ./prover
COPY zero_bin/leader ./leader

RUN \
    touch ops/src/lib.rs && \
    touch common/src/lib.rs && \
    touch rpc/src/lib.rs && \
    touch prover/src/lib.rs && \
    touch leader/src/main.rs && \
    touch coordinator/src/main.rs

RUN cargo build --verbose --release --bin coordinator

FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ca-certificates libjemalloc2 make libssl-dev
COPY --from=builder ./target/release/coordinator /usr/local/bin/coordinator
CMD ["coordinator"]
