#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

ENV_FILE=".env"
if [ ! -f "$ENV_FILE" ]; then
  echo "Missing $ENV_FILE." >&2
  echo "Run ./scripts/setup-devnet-env.sh to generate one from offckb's devnet accounts," >&2
  echo "or copy .env.example to $ENV_FILE and fill in your own values." >&2
  exit 1
fi
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

PROGRAM=./target/debug/vault-client

case "${1:-}" in
  create|withdraw)
    "$PROGRAM" "$@"
    ;;
  *)
    echo "usage: $0 create <ckb> | withdraw <tx_hash> <index> <ckb> [--short | --split a,b,...]"
    exit 1
    ;;
esac
