#!/usr/bin/env bash
# End-to-end demo of the sadaqah-vault contract via vault-client, against a
# local offckb devnet. Produces clear, screenshot-friendly step-by-step
# output for weekly reports.
#
# Usage: ./scripts/e2e-vault-client.sh
#
# Requires:
#   - a running devnet (`offckb node --daemon`)
#   - the contract deployed to it (`offckb deploy`, see deployment/devnet/sadaqah-lock)
#   - .env pointing at that deployment (run ./scripts/setup-devnet-env.sh if you don't have one)
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

echo "${BOLD}sadaqah-vault end-to-end demo (devnet)${RESET}"

step "0. Pre-flight checks"
if ! curl -s -X POST -H "Content-Type: application/json" \
     -d '{"id":1,"jsonrpc":"2.0","method":"get_tip_block_number","params":[]}' "$RPC" \
     | jq -e .result >/dev/null 2>&1; then
  fail "devnet RPC not reachable at $RPC (is 'offckb node' running?)"
fi
pass "devnet reachable at $RPC"

if [ ! -f .env ]; then
  echo "no .env found, generating one via scripts/setup-devnet-env.sh..."
  ./scripts/setup-devnet-env.sh
fi
pass ".env present"

step "1. Build vault-client"
cargo build -p vault-client --quiet
pass "vault-client built"

step "2. Create a sadaqah vault with 10,000 CKB"
CREATE_OUT=$(./run-client.sh create 10000)
echo "$CREATE_OUT"
CREATE_TX=$(echo "$CREATE_OUT" | extract_tx)
[ -n "$CREATE_TX" ] || fail "couldn't parse tx hash from create output"
echo "waiting for commitment..."
./scripts/wait-for-tx.sh "$CREATE_TX" >/dev/null
pass "vault created: $CREATE_TX (output #0)"

step "3. Withdraw 4,000 CKB, splitting the donation across two approved charities"
WITHDRAW_OUT=$(./run-client.sh withdraw "$CREATE_TX" 0 4000 --split 61,62)
echo "$WITHDRAW_OUT"
WITHDRAW_TX=$(echo "$WITHDRAW_OUT" | extract_tx)
[ -n "$WITHDRAW_TX" ] || fail "couldn't parse tx hash from withdraw output"
echo "waiting for commitment..."
./scripts/wait-for-tx.sh "$WITHDRAW_TX" >/dev/null
pass "split withdrawal accepted on-chain: $WITHDRAW_TX"

step "4. Attempt to underpay the charity share (the contract should reject this)"
set +e
SHORT_OUT=$(./run-client.sh withdraw "$WITHDRAW_TX" 0 5000 --short 2>&1)
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
FINAL_OUT=$(./run-client.sh withdraw "$WITHDRAW_TX" 0 1000)
echo "$FINAL_OUT"
FINAL_TX=$(echo "$FINAL_OUT" | extract_tx)
[ -n "$FINAL_TX" ] || fail "couldn't parse tx hash from final withdraw output"
echo "waiting for commitment..."
./scripts/wait-for-tx.sh "$FINAL_TX" >/dev/null
pass "final withdrawal accepted on-chain: $FINAL_TX"

step "Summary"
echo "  1. create                         -> $CREATE_TX"
echo "  2. withdraw --split (2 charities) -> $WITHDRAW_TX"
echo "  3. withdraw --short               -> rejected on-chain (error code 8, as expected)"
echo "  4. withdraw (normal)              -> $FINAL_TX"
echo
echo "${GREEN}${BOLD}All sadaqah-vault end-to-end scenarios passed.${RESET}"
