#!/usr/bin/env bash
# Generates a devnet .env for vault-client without ever hardcoding a private
# key in the repo: owner/charity keys are pulled live from offckb's built-in
# devnet dev accounts, and the contract code location is read from the latest
# deployment migration.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

command -v offckb >/dev/null 2>&1 || {
  echo "error: offckb is required (npm i -g offckb) to fetch devnet dev accounts." >&2
  exit 1
}
command -v jq >/dev/null 2>&1 || {
  echo "error: jq is required to parse deployment migration files." >&2
  exit 1
}

# offckb account index used for the vault owner, and for each approved charity
# (first entry is the "primary" charity used by default/--short withdrawals).
OWNER_IDX=18
CHARITY_IDXS=(17 16 15)

ACCOUNTS_RAW=$(offckb accounts --show-private-keys 2>/dev/null | awk '
  /^- "#":/    { idx = $3 }
  /^address:/  { addr = $2 }
  /^privkey:/  { priv = $2; printf "%s %s %s\n", idx, addr, priv }
')

field() { # $1 = account index, $2 = awk field (2=address, 3=privkey)
  echo "$ACCOUNTS_RAW" | awk -v i="$1" -v f="$2" '$1 == i { print $f }'
}

OWNER_PRIVKEY=$(field "$OWNER_IDX" 3)
if [ -z "$OWNER_PRIVKEY" ]; then
  echo "error: couldn't find offckb account #$OWNER_IDX; is offckb's devnet account list unchanged?" >&2
  exit 1
fi

CHARITY_ADDRS=()
for idx in "${CHARITY_IDXS[@]}"; do
  addr=$(field "$idx" 2)
  if [ -z "$addr" ]; then
    echo "error: couldn't find offckb account #$idx" >&2
    exit 1
  fi
  CHARITY_ADDRS+=("$addr")
done
CHARITY_ADDRESSES=$(IFS=,; echo "${CHARITY_ADDRS[*]}")

MIGRATIONS_DIR="deployment/devnet/sadaqah-lock/migrations"
LATEST_MIGRATION=$(ls -1 "$MIGRATIONS_DIR"/*.json 2>/dev/null | sort | tail -n1 || true)
if [ -z "$LATEST_MIGRATION" ]; then
  echo "error: no migration found under $MIGRATIONS_DIR; deploy the contract first (offckb deploy)." >&2
  exit 1
fi

VAULT_CODE_TX_HASH=$(jq -r '.cell_recipes[0].tx_hash' "$LATEST_MIGRATION")
VAULT_CODE_HASH=$(jq -r '.cell_recipes[0].data_hash' "$LATEST_MIGRATION")

cat > .env <<EOF
CKB_RPC=http://127.0.0.1:8114
OWNER_PRIVKEY=${OWNER_PRIVKEY}
CHARITY_ADDRESSES=${CHARITY_ADDRESSES}
VAULT_CODE_HASH=${VAULT_CODE_HASH}
VAULT_CODE_TX_HASH=${VAULT_CODE_TX_HASH}
VAULT_CODE_INDEX=0
VAULT_HASH_TYPE=data1
RATE_BPS=250
EOF
chmod 600 .env

echo "wrote .env (owner=#$OWNER_IDX, charities=#${CHARITY_IDXS[*]})"
echo "vault code: ${VAULT_CODE_HASH} @ ${VAULT_CODE_TX_HASH}"
