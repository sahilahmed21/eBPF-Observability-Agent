# Multi-stage image for the eBPF observability agent (Phase 4).
#
# Build (from repo root, on Linux/WSL):
#   docker build -f deploy/Dockerfile -t ebpf-obs-agent:latest .
#
# Runtime needs a host kernel with BTF; this image does not ship a kernel.

FROM rust:1.88-bookworm AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang llvm lld libelf-dev zlib1g-dev curl zstd \
    && rm -rf /var/lib/apt/lists/*
RUN rustup toolchain install nightly --component rust-src \
    && rustup component add rust-src
# Install a prebuilt bpf-linker binary rather than `cargo install bpf-linker`:
# building it from source requires an exact match between the system LLVM
# and the one the Rust toolchain was built against, which the plain
# clang/llvm apt packages don't guarantee (see aya-rs/bpf-linker#266).
RUN curl -fL -o /tmp/bpf-linker.tar.zst \
      https://github.com/aya-rs/bpf-linker/releases/latest/download/bpf-linker-x86_64-unknown-linux-musl.tar.zst \
    && tar --zstd -xf /tmp/bpf-linker.tar.zst -C /usr/local/bin \
    && rm /tmp/bpf-linker.tar.zst \
    && chmod +x /usr/local/bin/bpf-linker \
    && bpf-linker --version
COPY . .
ENV CARGO_TARGET_DIR=/tmp/target
RUN cargo build --release -p obsagent

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /tmp/target/release/obsagent /usr/local/bin/obsagent
ENV OBSAGENT_HEADLESS=1
ENTRYPOINT ["/usr/local/bin/obsagent"]
