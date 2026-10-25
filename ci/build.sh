#!/bin/sh

. ./ci/preamble.sh

# TODO
#if test "$GITHUB_ACTIONS" = "true" && test "$GITHUB_REF_TYPE" != "tag"; then
#    exit 0
#fi

set -e
target=x86_64-unknown-linux-musl
cargo build \
    --quiet \
    --release \
    --target "$target" \
    --package kpea-cli
mv cargo-fast-install.jsonl cargo-fast-install.jsonl.old
cargo-fast-install \
    --generate \
    --release \
    --target "$target" \
    --package kpea-cli \
    --url "{crate.repository}/releases/{crate.version}/download/kpea-{crate.version}.tar.gz"
if ! diff cargo-fast-install.jsonl.old cargo-fast-install.jsonl; then
    cat cargo-fast-install.jsonl
    exit 1
fi
