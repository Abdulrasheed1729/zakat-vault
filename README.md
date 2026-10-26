# sadaqah-vault

A CKB on-chain script project implementing a **sadaqah vault**: a lock
script that lets an owner lock up capacity while committing to give a
configurable percentage of every withdrawal to one or more pre-approved
charities.

Unlike zakat, sadaqah is voluntary and has no fixed rate, nisab threshold,
holding period, or recipient category — so this contract doesn't try to
enforce religious compliance. It enforces exactly one rule the owner opts
into when creating the vault: *"whenever I withdraw, at least X% goes to a
charity I've pre-approved."*

See [`contracts/sadaqah-lock/README.md`](contracts/sadaqah-lock/README.md)
for the full args layout, semantics, and error codes.

## Project layout

- `contracts/sadaqah-lock` — the lock script implementation (RISC-V,
  `no_std`).
- `tests` — native (`std`) integration tests using
  [ckb-testtool](https://github.com/nervosnetwork/ckb-testtool).

## Building and testing

```
make build
make test
```

*This project was bootstrapped with [ckb-script-templates].*

[ckb-script-templates]: https://github.com/nervosnetwork/ckb-script-templates
