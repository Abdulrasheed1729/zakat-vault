#!/usr/bin/env bash
# Polls the devnet RPC until a transaction is committed (or times out).
# Usage: scripts/wait-for-tx.sh <tx_hash> [max_tries]
set -euo pipefail

TX="$1"
MAX_TRIES="${2:-30}"
RPC="${CKB_RPC:-http://127.0.0.1:8114}"

for i in $(seq 1 "$MAX_TRIES"); do
  STATUS=$(curl -s -X POST -H "Content-Type: application/json" \
    -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"get_transaction\",\"params\":[\"$TX\"]}" \
    "$RPC" | jq -r '.result.tx_status.status')
  echo "  [$i/$MAX_TRIES] $TX -> $STATUS"
  if [ "$STATUS" = "committed" ]; then
    exit 0
  fi
  sleep 2
done

echo "timed out waiting for $TX to commit" >&2
exit 1
