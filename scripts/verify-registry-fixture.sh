#!/usr/bin/env bash
set -euo pipefail
if [[ $# -ne 3 ]]; then
  echo "usage: $0 <TikTools checkout> <fixture directory> <root pin file>" >&2
  exit 2
fi
repo=$(realpath "$1")
fixture=$(realpath "$2")
pin=$(realpath "$3")
revision=983a375c5b702681a791061ca863cea9c542528d
[[ $(git -C "$repo" rev-parse HEAD) == "$revision" ]] || { echo "unexpected TikTools revision" >&2; exit 1; }
git -C "$repo" diff --quiet && git -C "$repo" diff --cached --quiet || { echo "TikTools checkout has tracked changes" >&2; exit 1; }
script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/src" "$work/scratch"
cp "$script_dir/../tests/registry-fixture-client/main.rs" "$work/src/main.rs"
cp "$repo/Cargo.lock" "$work/Cargo.lock"
esc=${repo//\\/\\\\}
esc=${esc//\"/\\\"}
cat > "$work/Cargo.toml" <<EOF
[package]
name = "cloud-registry-fixture-verifier"
version = "0.1.0"
edition = "2021"
[dependencies]
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util"] }
tiktools-plugin-package = { path = "$esc/crates/tiktools-plugin-package" }
tiktools-plugin-registry = { path = "$esc/crates/tiktools-plugin-registry" }
EOF
CARGO_TARGET_DIR="$repo/target" cargo run --offline --manifest-path "$work/Cargo.toml" -- "$fixture" "$pin" "$work/scratch"
