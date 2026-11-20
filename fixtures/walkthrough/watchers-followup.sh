#!/usr/bin/env bash
set -euo pipefail
: "${DATABASE_URL:?Use a fresh disposable database}"
repo=$(realpath -- "$(dirname -- "$0")/../..")
platform=$(realpath -- "$repo/../../..")
cargo run --locked --manifest-path "$platform/Cargo.toml" -p bilibi-web --example watchers-domains-walkthrough -- commerce
