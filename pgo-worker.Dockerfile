#############
# Build stage
#############
# - `/src` is the repo directory.
# - `/artifacts` is $CARGO_TARGET_DIR.
# - `/output` is where the binaries go.

ARG BUILD_BASE=rustlang/rust:nightly-bullseye-slim
FROM ${BUILD_BASE} AS build
ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDPLATFORM

# Install build dependencies.
RUN dpkg add --add-architecture arm64 && \
    apt-get update && apt-get install -y \
    # for jemalloc
    libjemalloc-dev \
    libjemalloc2 \
    make \
    # for openssl
    libssl-dev \
    pkg-config \
    # for cross compilation
    libssl-dev:arm64 \
    gcc-aarch64-linux-gnu \
    && rustup target add aarch64-unknown-linux-gnu \
    # clean the image
    python3 python3-pip \
    python3:arm64 python3-pip:arm64 \
    && rm -rf /var/lib/apt/lists/*


RUN cargo install cargo-pgo && pip3 install google-cloud-storage

ARG PROFILE=release
# forward the docker argument so that the script below can read it
ENV PROFILE=${PROFILE}

WORKDIR /src

COPY . .

# Build the application.
RUN \
    # cache artifacts and the cargo registry to speed up subsequent builds
    --mount=type=cache,target=/usr/local/cargo/registry/ \
    # run the build
    <<EOF
set -eux

TARGET=""
case ${TARGETARCH} in \
        arm64) TARGET="aarch64-unknown-linux-gnu" ;; \
        amd64) TARGET="x86_64-unknown-linux-gnu" ;; \
        *) exit 1 ;; \
esac

cargo pgo build -- --locked --bin worker "--target=${TARGET}"

EOF

# NOTE: the bucket name should be set WITHOUT the `gs://` prefix
#  BONUS NOTE: should we create a different bucket just for .profraw files?
ENV GCS_UPLOAD_BUCKET=zkevm-csv
ENV WORKER_PATH=./target/${TARGET}/release/worker
ENV PROFILE_DIRECTORY=./target/pgo-profiles/

# run the python wrapper, which will:
#   1. execute the pgo-worker binary
#   2. wait to receive a signal (either SIGTERM or SIGKILL), then sends a SIGTERM to the pgo-worker binary
#   3. upload the created pgo .profraw file to GCS
CMD ["python3", "pgo_worker_wrapper.py"]