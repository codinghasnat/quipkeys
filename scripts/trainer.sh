#!/usr/bin/env bash
# Build the engine for the browser and serve the typing trainer.
#   scripts/trainer.sh            -> http://localhost:8000
#   scripts/trainer.sh --build    -> just build trainer/quipkeys.wasm
set -euo pipefail
cd "$(dirname "$0")/.."
rustup target list --installed | grep -q wasm32-unknown-unknown || rustup target add wasm32-unknown-unknown
CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=true \
  cargo build --release --lib --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/quipkeys.wasm trainer/quipkeys.wasm
echo "built trainer/quipkeys.wasm ($(du -h trainer/quipkeys.wasm | cut -f1))"
[ "${1:-}" = "--build" ] && exit 0
echo "QuipKeys Trainer: http://localhost:${PORT:-8000}"
exec python3 -m http.server "${PORT:-8000}" --bind 127.0.0.1 -d trainer
