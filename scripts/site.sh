#!/bin/sh
# Builds the docs site into site/dist: the pages, plus each example in
# site/examples compiled to its own wasm under wasm/. Needs the
# wasm32-unknown-unknown target and wasm-bindgen-cli at the wasm-bindgen
# version in Cargo.lock; uses wasm-opt when installed. Serve with:
#   python3 -m http.server -d site/dist
set -e
cd "$(dirname "$0")/.."
out=site/dist
rm -rf "$out"
cargo run -p dfui-site --release -- "$out"
cargo build -p dfui-site --examples --profile web --target wasm32-unknown-unknown
for wasm in target/wasm32-unknown-unknown/web/examples/*.wasm; do
    name=$(basename "$wasm" .wasm)
    # Cargo also leaves hashed copies (name-<hash>.wasm).
    case $name in *-*) continue ;; esac
    wasm-bindgen --target web --no-typescript --out-dir "$out/wasm" "$wasm"
    if command -v wasm-opt >/dev/null; then
        wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
            --enable-sign-ext --enable-reference-types --enable-multivalue \
            "$out/wasm/${name}_bg.wasm" -o "$out/wasm/${name}_bg.wasm"
    fi
done
