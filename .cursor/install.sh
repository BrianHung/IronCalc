#!/usr/bin/env bash
# Idempotent bootstrap for the IronCalc Cloud Agent environment.
# Prepares the Rust + WASM toolchain and builds every artifact the web app
# (wasm bindings -> workbook widget -> frontend) and the Rocket API depend on.
set -euo pipefail

WASM_PACK_VERSION="0.13.1"

echo "==> Ensuring Rust toolchain (stable) and wasm target"
# A transitive dependency requires the 2024 edition, so the default toolchain
# must be recent stable rather than whatever the base image ships.
rustup toolchain install stable --profile minimal --no-self-update >/dev/null 2>&1 || true
rustup default stable
rustup target add wasm32-unknown-unknown

echo "==> Ensuring wasm-pack is installed"
if ! command -v wasm-pack >/dev/null 2>&1; then
  curl -sSL "https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl.tar.gz" -o /tmp/wasm-pack.tar.gz
  tar -xzf /tmp/wasm-pack.tar.gz -C /tmp
  sudo install -m0755 /tmp/wasm-pack-v${WASM_PACK_VERSION}-x86_64-unknown-linux-musl/wasm-pack /usr/local/bin/wasm-pack
fi

echo "==> Ensuring caddy is installed"
if ! command -v caddy >/dev/null 2>&1; then
  curl -sSL "https://caddyserver.com/api/download?os=linux&arch=amd64" -o /tmp/caddy
  sudo install -m0755 /tmp/caddy /usr/local/bin/caddy
fi

echo "==> Building WASM bindings (bindings/wasm/pkg)"
make -C bindings/wasm

echo "==> Building the workbook widget (@ironcalc/workbook)"
(cd webapp/IronCalc && npm install && npm run build)

echo "==> Installing frontend app dependencies"
(cd webapp/app.ironcalc.com/frontend && npm install)

echo "==> Pre-building the Rocket API server"
(cd webapp/app.ironcalc.com/server && cargo build)

echo "==> Install complete"
