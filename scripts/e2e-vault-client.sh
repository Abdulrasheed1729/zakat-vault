#!/usr/bin/env bash
# End-to-end demo of the sadaqah-vault contract via vault-client. Works
# against any network (devnet, testnet) that the current config points at.
# Produces clear, screenshot-friendly step-by-step output for reports, and
# is also used as the testnet smoke test in CI (see .github/workflows).
#
# Usage: ./scripts/e2e-vault-client.sh
#
# Requires contract config, either via:
#   - .env at the project root (local/devnet: run ./scripts/setup-devnet-env.sh), or
#   - the same variables already exported in the environment (CI/testnet)
#
# Tunable via env vars (defaults are devnet-friendly; override for smaller
# testnet balances): CREATE_AMOUNT_CKB, WITHDRAW_AMOUNT_CKB, SPLIT_SPEC,
# SHORT_AMOUNT_CKB, FINAL_AMOUNT_CKB, WAIT_MAX_TRIES, WAIT_INTERVAL_SECS.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

BOLD="$(tput bold 2>/dev/null || true)"
RESET="$(tput sgr0 2>/dev/null || true)"
GREEN="$(tput setaf 2 2>/dev/null || true)"
RED="$(tput setaf 1 2>/dev/null || true)"
CYAN="$(tput setaf 6 2>/dev/null || true)"

step() { echo; echo "${BOLD}${CYAN}=== $* ===${RESET}"; }
pass() { echo "${GREEN}PASS:${RESET} $*"; }
fail() { echo "${RED}FAIL:${RESET} $*"; exit 1; }

extract_tx() { grep -oE '0x[0-9a-f]{64}' | head -n1; }

RPC="${CKB_RPC:-http://127.0.0.1:8114}"
WAIT_MAX_TRIES="${WAIT_MAX_TRIES:-30}"
WAIT_INTERVAL_SECS="${WAIT_INTERVAL_SECS:-2}"

CREATE_AMOUNT_CKB="${CREATE_AMOUNT_CKB:-10000}"
WITHDRAW_AMOUNT_CKB="${WITHDRAW_AMOUNT_CKB:-4000}"
SPLIT_SPEC="${SPLIT_SPEC:-61,62}"
SHORT_AMOUNT_CKB="${SHORT_AMOUNT_CKB:-5000}"
FINAL_AMOUNT_CKB="${FINAL_AMOUNT_CKB:-1000}"

wait_for() { ./scripts/wait-for-tx.sh "$1" "$WAIT_MAX_TRIES" "$WAIT_INTERVAL_SECS" >/dev/null; }

echo "${BOLD}sadaqah-vault end-to-end demo${RESET} (RPC: $RPC)"

step "0. Pre-flight checks"
if ! curl -s -X POST -H "Content-Type: application/json" \
     -d '{"id":1,"jsonrpc":"2.0","method":"get_tip_block_number","params":[]}' "$RPC" \
     | jq -e .result >/dev/null 2>&1; then
  fail "node RPC not reachable at $RPC"
fi
pass "node reachable at $RPC"

# Only auto-generate a devnet .env when nothing else has already configured
# the client (e.g. CI exporting config as job env for testnet).
if [ -z "${OWNER_PRIVKEY:-}" ] && [ ! -f .env ]; then
  echo "no .env found and no config in the environment, generating .env via scripts/setup-devnet-env.sh..."
  ./scripts/setup-devnet-env.sh
fi
pass "client config present"

step "1. Build vault-client"
cargo build -p vault-client --quiet
pass "vault-client built"

step "2. Create a sadaqah vault with ${CREATE_AMOUNT_CKB} CKB"
CREATE_OUT=$(./run-client.sh create "$CREATE_AMOUNT_CKB")
echo "$CREATE_OUT"
CREATE_TX=$(echo "$CREATE_OUT" | extract_tx)
[ -n "$CREATE_TX" ] || fail "couldn't parse tx hash from create output"
echo "waiting for commitment..."
wait_for "$CREATE_TX"
pass "vault created: $CREATE_TX (output #0)"

step "3. Withdraw ${WITHDRAW_AMOUNT_CKB} CKB, splitting the donation across two approved charities"
WITHDRAW_OUT=$(./run-client.sh withdraw "$CREATE_TX" 0 "$WITHDRAW_AMOUNT_CKB" --split "$SPLIT_SPEC")
echo "$WITHDRAW_OUT"
WITHDRAW_TX=$(echo "$WITHDRAW_OUT" | extract_tx)
[ -n "$WITHDRAW_TX" ] || fail "couldn't parse tx hash from withdraw output"
echo "waiting for commitment..."
wait_for "$WITHDRAW_TX"
pass "split withdrawal accepted on-chain: $WITHDRAW_TX"

step "4. Attempt to underpay the charity share (the contract should reject this)"
set +e
SHORT_OUT=$(./run-client.sh withdraw "$WITHDRAW_TX" 0 "$SHORT_AMOUNT_CKB" --short 2>&1)
SHORT_STATUS=$?
set -e
echo "$SHORT_OUT"
if [ "$SHORT_STATUS" -eq 0 ]; then
  fail "underpaid withdrawal was accepted on-chain -- this should NOT happen"
fi
if echo "$SHORT_OUT" | grep -q "error code 8"; then
  pass "underpaid withdrawal correctly rejected (CharityShortPaid, error code 8)"
else
  fail "withdrawal failed, but not with the expected CharityShortPaid error"
fi

step "5. Withdraw correctly again, to show the vault is still usable after the rejection"
FINAL_OUT=$(./run-client.sh withdraw "$WITHDRAW_TX" 0 "$FINAL_AMOUNT_CKB")
echo "$FINAL_OUT"
FINAL_TX=$(echo "$FINAL_OUT" | extract_tx)
[ -n "$FINAL_TX" ] || fail "couldn't parse tx hash from final withdraw output"
echo "waiting for commitment..."
wait_for "$FINAL_TX"
pass "final withdrawal accepted on-chain: $FINAL_TX"

step "Summary"
echo "  1. create                         -> $CREATE_TX"
echo "  2. withdraw --split (2 charities) -> $WITHDRAW_TX"
echo "  3. withdraw --short               -> rejected on-chain (error code 8, as expected)"
echo "  4. withdraw (normal)              -> $FINAL_TX"
echo
echo "${GREEN}${BOLD}All sadaqah-vault end-to-end scenarios passed.${RESET}"
