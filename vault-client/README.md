# vault-client

A small Rust CLI for creating and withdrawing from `sadaqah-lock` vault
cells on a CKB devnet (or testnet, with the right config). It exists to
exercise the contract end-to-end against a real node, as a complement to
the unit tests in `tests/`.

## Setup

1. Have a devnet running and the contract deployed to it. See
   `deployment/devnet/sadaqah-lock/` for the deployment config/migrations
   (produced by `offckb deploy`).
2. Generate a `.env` for the client:

   ```sh
   ./scripts/setup-devnet-env.sh
   ```

   This pulls the owner/charity keys live from `offckb accounts
   --show-private-keys` (devnet-only, well-known dev keys — never real
   secrets) and reads the deployed code hash/tx hash from the latest
   migration file under `deployment/devnet/sadaqah-lock/migrations/`. It
   writes `.env` at the project root, which is gitignored and never
   committed.

   Alternatively, copy `.env.example` to `.env` and fill in the values by
   hand (useful for testnet, or if you want specific accounts).
3. Build the client:

   ```sh
   cargo build -p vault-client
   ```

All commands below are run through `./run-client.sh`, which loads `.env`
and forwards to the built binary.

## Commands

### `create <ckb_amount>`

Locks `<ckb_amount>` CKB into a new vault cell, using the owner and
approved charities from `.env`.

```sh
./run-client.sh create 10000
```

Prints the transaction hash. The new vault cell is output index `0` of
that transaction — you'll need both for `withdraw`.

### `withdraw <vault_tx_hash> <vault_index> <ckb_amount>`

Withdraws `<ckb_amount>` CKB from the vault cell at
`<vault_tx_hash>:<vault_index>`, automatically paying the required
donation share (`ceil(withdrawn * RATE_BPS / 10000)`) to the first
approved charity.

```sh
./run-client.sh withdraw 0xabc...123 0 4000
```

The vault's remaining balance (if any) is output index `0` of the new
transaction, so you can chain further withdrawals from it the same way.

#### `--split a,b,...`

Pays specific CKB amounts to specific approved charities instead of
sending the full share to the first one, exercising the contract's
multi-charity support (up to 4 approved charities, any subset payable per
withdrawal). Amounts are positional, matching the order of
`CHARITY_ADDRESSES` in `.env`.

```sh
# Pay 61 CKB to the 1st approved charity and 62 CKB to the 2nd
./run-client.sh withdraw 0xabc...123 0 4000 --split 61,62
```

Each non-zero amount must be at least 61 CKB (the minimum capacity for a
plain cell) — the client checks this up front with a clear error, rather
than letting it fail somewhere else in the transaction.

#### `--short`

Deliberately pays only 70% of the required share, to confirm the
contract correctly rejects the withdrawal (error code 8,
`CharityShortPaid`). Useful for testing/demos. Mutually exclusive with
`--split`.

```sh
./run-client.sh withdraw 0xabc...123 0 5000 --short
```

## Environment variables

Set in `.env` (see `.env.example` for the template):

| Variable             | Description                                                                 |
|----------------------|------------------------------------------------------------------------------|
| `CKB_RPC`            | Node RPC URL (default `http://127.0.0.1:8114`)                               |
| `OWNER_PRIVKEY`      | Owner's private key, hex. **Devnet only** — never put a real key here.       |
| `CHARITY_ADDRESSES`  | Comma-separated list of 1–4 approved charity addresses                       |
| `VAULT_CODE_HASH`    | Deployed contract's code hash                                                |
| `VAULT_CODE_TX_HASH` | Tx hash of the deployed code cell                                            |
| `VAULT_CODE_INDEX`   | Output index of the code cell (default `0`)                                  |
| `VAULT_HASH_TYPE`    | `data1` (default), `data`, or `type` — must match how the cell was deployed  |
| `RATE_BPS`           | Donation rate in basis points (default `250` = 2.5%)                        |

## End-to-end demo

`scripts/e2e-vault-client.sh` runs a full scripted walkthrough (create →
split withdrawal → rejected underpayment → successful withdrawal) against
whatever node the current config points at, with clear pass/fail output —
handy for reports or sanity-checking after a contract change:

```sh
./scripts/e2e-vault-client.sh
```

It reads config from `.env` if present, falling back to whatever is already
in the environment (which is how CI runs it — see below). The CKB amounts
used at each step are overridable via env vars (`CREATE_AMOUNT_CKB`,
`WITHDRAW_AMOUNT_CKB`, `SPLIT_SPEC`, `SHORT_AMOUNT_CKB`,
`FINAL_AMOUNT_CKB`) if you need smaller amounts for a less-funded account.

## Testnet CI

`.github/workflows/testnet-e2e.yml` runs this same e2e script against CKB
testnet, manually triggered from the Actions tab (it spends real faucet
funds, so it isn't run on every push). One-time setup:

1. Deploy the contract to testnet yourself with a funded key:
   ```sh
   offckb deploy --network testnet --privkey-file <path-to-your-key>
   ```
   This produces `deployment/testnet/sadaqah-lock/` — commit it.
2. Fund an owner account and one or more charity addresses from the
   [testnet faucet](https://faucet.nervos.org).
3. In the repo's **Settings → Secrets and variables → Actions**, add:
   - Secrets: `TESTNET_OWNER_PRIVKEY`, `TESTNET_CHARITY_ADDRESSES`
   - Variables: `TESTNET_VAULT_CODE_HASH`, `TESTNET_VAULT_CODE_TX_HASH`,
     and optionally `TESTNET_RPC_URL`

See the comments at the top of the workflow file for the full details.

## Notes

- `create` doesn't invoke the vault's lock script at all (it only guards
  *spending*), so it always succeeds as long as the owner has enough
  capacity.
- The owner's input in a `withdraw` transaction both authorizes the
  withdrawal (per the contract's `check_owner`) and pays the transaction
  fee.
- This client intentionally keeps transaction-building logic minimal and
  explicit (no `ckb-sdk` transaction builder, just raw `TransactionBuilder`
  + manual signing) to make it easy to read alongside the contract's
  `README.md` and verify they agree on the args layout and semantics.
