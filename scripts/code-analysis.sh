#!/bin/bash

cargo fmt --all -- --check && \
    cargo clippy --all-targets -- -D warnings && \
    cargo check --bins && \
    cargo doc --no-deps --all && \
    cargo test --all
