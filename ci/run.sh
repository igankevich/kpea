#!/bin/sh

exec docker run --rm -it -v "$PWD":/src --workdir /src -e CARGO_HOME=/src/.cargo-docker \
    ghcr.io/igankevich/kpea-ci:latest "$@"
