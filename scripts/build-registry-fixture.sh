#!/usr/bin/env bash
# Generate a disposable test registry with TikTools' own tooling. Never use for real keys.
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 <TikTools PR 43 checkout> <new output directory>" >&2
  exit 2
fi
repo=$(realpath "$1")
output=$2
revision=983a375c5b702681a791061ca863cea9c542528d
if [[ $(git -C "$repo" rev-parse HEAD) != "$revision" ]]; then
  echo "unexpected TikTools revision; expected $revision" >&2
  exit 1
fi
if ! git -C "$repo" diff --quiet || ! git -C "$repo" diff --cached --quiet; then
  echo "TikTools checkout has tracked changes" >&2
  exit 1
fi
if [[ -e "$output" || -e "${output}.root-pin.json" ]]; then
  echo "output must not already exist" >&2
  exit 1
fi
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
umask 077
export CARGO_TARGET_DIR="$repo/target"
cargo build --locked --manifest-path "$repo/Cargo.toml" -p tiktools-plugin-devkit --bin tiktools-plugin
cli="$CARGO_TARGET_DIR/debug/tiktools-plugin"
"$cli" new acme.hooks --template declarative --dir "$work/plugin"
"$cli" keygen --id fixture-root --role root --out "$work/keys"
"$cli" keygen --id fixture-index --role index --out "$work/keys"
"$cli" keygen --id fixture-acme --publisher acme --out "$work/keys"
"$cli" pack "$work/plugin" --out-dir "$work/dist" --sign "$work/keys/fixture-acme.key.json"
"$cli" index init "$work/registry" --name fixture --root-key "$work/keys/fixture-root.key.json" --index-key "$work/keys/fixture-index.key.json" --delegate "$work/keys/fixture-acme.pub.json"
"$cli" publish "$work/dist/acme.hooks-0.1.0.plugin" --registry "$work/registry" --index-key "$work/keys/fixture-index.key.json"
mkdir -p -- "$output"
cp -a -- "$work/registry/." "$output/"
cp -- "$work/keys/fixture-root.pub.json" "${output}.root-pin.json"
echo "Disposable test registry written to $output. Its keys are not production trust anchors."
