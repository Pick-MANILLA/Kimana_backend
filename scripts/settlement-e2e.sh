#!/usr/bin/env bash
# Backend-driven settlement against kimana_contract's `make e2e` Anvil setup:
# starts Anvil, deploys and seeds the vault with LocalE2E.s.sol, unpauses it
# (the scenario ends paused), then runs tests/settlement_anvil.rs (the client)
# and tests/settlement_listener.rs (the event listener, which also needs Postgres).
#
#   KIMANA_CONTRACT_DIR=../kimana_contract bash scripts/settlement-e2e.sh
#
# Needs Foundry (anvil, forge, cast), `make install` already run in the contract
# repo, and Postgres at DATABASE_URL (docker compose up -d).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONTRACT_DIR="$(cd "${KIMANA_CONTRACT_DIR:-$ROOT/../kimana_contract}" && pwd)"
PORT="${ANVIL_PORT:-8545}"
RPC="http://127.0.0.1:${PORT}"
LOG="$(mktemp)"
# LocalE2E.s.sol's admin: Anvil's public default account #1.
ADMIN_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

anvil --port "$PORT" --silent &
ANVIL_PID=$!
trap 'kill $ANVIL_PID 2>/dev/null || true; rm -f "$LOG"' EXIT
for _ in $(seq 1 50); do cast block-number --rpc-url "$RPC" >/dev/null 2>&1 && break; sleep 0.2; done

(cd "$CONTRACT_DIR" && forge script script/LocalE2E.s.sol --rpc-url "$RPC" --broadcast --slow) > "$LOG" 2>&1 \
  || { cat "$LOG"; exit 1; }
VAULT=$(grep -E '^\s*VAULT ' "$LOG" | awk '{print $2}')
[ -n "$VAULT" ] || { cat "$LOG"; echo "could not read the vault address"; exit 1; }
echo "vault=$VAULT"

cast send "$VAULT" "unpause()" --private-key "$ADMIN_PK" --rpc-url "$RPC" >/dev/null

# One thread: every test signs as the same operator, and parallel sends would race on the nonce.
cd "$ROOT"
SETTLEMENT_RPC_URL="$RPC" SETTLEMENT_VAULT_ADDRESS="$VAULT" \
  cargo test --test settlement_anvil --test settlement_listener -- --ignored --test-threads=1
