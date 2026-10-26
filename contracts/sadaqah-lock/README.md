# sadaqah-lock

A CKB lock script that turns a cell into a **sadaqah vault**: a self-imposed
commitment that whenever capacity leaves the vault, a configurable share of
it is donated to one or more pre-approved charities.

This is deliberately *not* a zakat-compliance checker (no nisab, no hawl, no
fixed asnaf categories) — sadaqah is voluntary and flexible, so the contract
only enforces the one rule the owner opts into at vault-creation time: *"pay
at least X% to charity whenever I withdraw."*

## Args layout

The lock script args are a flat, fixed-header + variable-length byte layout:

| Field            | Size             | Description                                  |
|------------------|------------------|-----------------------------------------------|
| `owner_lock_hash`| 32 bytes         | Blake2b hash of the owner's lock script       |
| `rate_bps`       | 2 bytes (LE u16) | Required donation rate in basis points (1–10000) |
| `charity_count`  | 1 byte           | Number of approved charities, `N` (1–4)       |
| `charity_lock_hash[1..=N]` | 32 bytes each | Blake2b hash of each approved charity's lock script |

Total args length = `35 + 32*N` bytes (67–163 bytes depending on `N`).

## Semantics

- **Owner authorization**: the transaction must include at least one input
  cell whose lock hash equals `owner_lock_hash`. This proves the owner's
  signature via their own lock script (a standard "lock dependency" pattern)
  — the vault lock itself does not do any signature verification.
- **Withdrawal amount**: computed as
  `sum(capacity of this script's cells in Inputs) - sum(capacity of this
  script's cells in Outputs)`, floored at zero. Top-ups, merges, or
  rearrangements that don't reduce the vault's total capacity require no
  charity payment.
- **Required charity share**: `ceil(withdrawn * rate_bps / 10000)`, rounded
  up so the charity is never short-changed by integer division.
- **Charity payment**: the owner may pay the required share to **any subset**
  of the approved charity locks, in any split. Payments to a lock that is
  *not* in the approved list do not count.
- **Recipient flexibility**: the approved charity set is fixed for the
  lifetime of a given vault lock (it's part of the args, and therefore part
  of the script hash). To support a different set of charities, create a new
  vault cell with new args — transferring funds into it is itself a
  withdrawal from the old vault and is subject to the old vault's charity
  rule.

## Errors

See [`src/error.rs`](src/error.rs) for the full list of error codes. Notable
ones beyond standard syscall errors:

- `BadRate`: `rate_bps` is `0` or greater than `10000`.
- `BadCharityCount`: `charity_count` is `0`, greater than `4`, or doesn't
  match the actual args length.
- `DuplicateCharity`: the same charity lock hash appears more than once in
  the args.
- `OwnerNotAuthorized`: no input cell carries the owner's lock hash.
- `CharityShortPaid`: the sum of outputs paid to approved charities is less
  than the required share of the withdrawn amount.
- `MathOverflow`: the withdrawn-amount × rate multiplication overflowed
  (practically unreachable given CKB's total capacity supply, but handled
  explicitly rather than reusing an unrelated error code).

## Debug logging

Build with `--features enable_log` to emit a debug line with the computed
`withdrawn`, `required`, and `paid` values via `ckb_std::debug!`. This only
has an effect in debug-assertion builds (see `ckb_std::debug!` docs) and has
no effect on production verification logic.

*This contract was bootstrapped with [ckb-script-templates].*

[ckb-script-templates]: https://github.com/nervosnetwork/ckb-script-templates
