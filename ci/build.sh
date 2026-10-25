#!/bin/sh

. ./ci/preamble.sh

set -e
target=x86_64-unknown-linux-musl
rustup toolchain add "$RUST_VERSION" --target "$target"
rustup default "$RUST_VERSION"
cargo build \
    --quiet \
    --release \
    --target "$target" \
    --package kpea-cli
cargo-fast-install \
    --generate \
    --release \
    --target "$target" \
    --package kpea-cli \
    --url "{crate.repository}/releases/{crate.version}/download/kpea-{crate.version}.tar.gz"
