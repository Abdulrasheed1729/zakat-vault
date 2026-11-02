// src/main.rs for a small Rust client: create a sadaqah-vault cell and withdraw from it on devnet.
//
// Usage:
//   cargo run -- create 10000                              # lock 10,000 CKB into a new vault
//   cargo run -- withdraw <vault_tx_hash> 0 4000            # withdraw 4,000 CKB, paying the full
//                                                            # required share to the first approved charity
//   cargo run -- withdraw <vault_tx_hash> 0 4000 --short    # same, but underpay the charity (should be rejected)
//   cargo run -- withdraw <vault_tx_hash> 0 4000 --split 15,10   # split the required share across the
//                                                            # first two approved charities (15k + 10k CKB)
//
// Environment variables (see .env.example, or run scripts/setup-devnet-env.sh):
//   CKB_RPC            node RPC url (default http://127.0.0.1:8114)
//   OWNER_PRIVKEY      owner's private key, hex (DEVNET ONLY)
//   CHARITY_ADDRESSES  comma-separated list of 1-4 approved charity addresses
//   VAULT_CODE_TX_HASH tx hash of your deployed contract code cell
//   VAULT_CODE_INDEX   output index of the code cell (default 0)
//   VAULT_CODE_HASH    code hash printed by the deployment
//   VAULT_HASH_TYPE    "data1" (default), "data", or "type"
//   RATE_BPS           donation rate in basis points (default 250 = 2.5%)
//
// Args layout (must match your contract README):
//   owner_lock_hash (32) | rate_bps (2, LE) | charity_count (1) | charity_lock_hash (32 each)

use std::{env, fmt::Debug, str::FromStr};

use ckb_hash::{blake2b_256, new_blake2b};
use ckb_jsonrpc_types::OutputsValidator;
use ckb_sdk::{
    Address,
    constants::SIGHASH_TYPE_HASH,
    rpc::CkbRpcClient,
    traits::{CellCollector, CellQueryOptions, DefaultCellCollector, ValueRangeOption},
};
use ckb_types::{
    H256,
    bytes::Bytes,
    core::{DepType, ScriptHashType, TransactionBuilder, TransactionView},
    packed::{CellDep, CellInput, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};

const CKB: u64 = 100_000_000; // 1 CKB in shannons
const MIN_CELL: u64 = 61 * CKB; // smallest cell with a standard lock
const FEE: u64 = 100_000; // 0.001 CKB, a fixed fee that is plenty for these small txs
const MAX_CHARITIES: usize = 4;

type R<T> = Result<T, String>;

fn e<E: Debug>(err: E) -> String {
    format!("{err:?}")
}

// ---------- Configuration ----------

struct Cfg {
    rpc_url: String,
    owner_key: SecretKey,
    owner_lock: Script,
    charity_locks: Vec<Script>,
    code_hash: H256,
    hash_type: ScriptHashType,
    code_out_point: OutPoint,
    rate_bps: u16,
}

fn env_req(key: &str) -> R<String> {
    env::var(key).map_err(|_| format!("please set {key}"))
}

fn h256(s: &str) -> R<H256> {
    H256::from_str(s.trim_start_matches("0x")).map_err(e)
}

fn load_cfg() -> R<Cfg> {
    let rpc_url = env::var("CKB_RPC").unwrap_or_else(|_| "http://127.0.0.1:8114".into());

    // Owner key -> default (secp256k1-blake160) lock script
    let key = h256(&env_req("OWNER_PRIVKEY")?)?;
    let owner_key = SecretKey::from_slice(key.as_bytes()).map_err(e)?;
    let pubkey = PublicKey::from_secret_key(&Secp256k1::new(), &owner_key);
    let owner_lock = Script::new_builder()
        .code_hash(SIGHASH_TYPE_HASH.pack())
        .hash_type(ScriptHashType::Type)
        .args(Bytes::from(blake2b_256(pubkey.serialize())[..20].to_vec()).pack())
        .build();

    // Approved charity addresses -> lock scripts
    let charity_addresses = env_req("CHARITY_ADDRESSES")?;
    let mut charity_locks = Vec::new();
    for addr_str in charity_addresses.split(',') {
        let addr_str = addr_str.trim();
        if addr_str.is_empty() {
            continue;
        }
        let addr = Address::from_str(addr_str).map_err(e)?;
        charity_locks.push(Script::from(addr.payload()));
    }
    if charity_locks.is_empty() {
        return Err("CHARITY_ADDRESSES must list at least one address".into());
    }
    if charity_locks.len() > MAX_CHARITIES {
        return Err(format!(
            "CHARITY_ADDRESSES lists more than {MAX_CHARITIES} charities"
        ));
    }
    let mut hashes: Vec<_> = charity_locks.iter().map(|l| l.calc_script_hash()).collect();
    hashes.sort();
    hashes.dedup();
    if hashes.len() != charity_locks.len() {
        return Err("CHARITY_ADDRESSES contains duplicate charities".into());
    }

    // Where your contract code lives
    let code_hash = h256(&env_req("VAULT_CODE_HASH")?)?;
    let hash_type = match env::var("VAULT_HASH_TYPE")
        .unwrap_or_else(|_| "data1".into())
        .as_str()
    {
        "data" => ScriptHashType::Data,
        "data1" => ScriptHashType::Data1,
        "type" => ScriptHashType::Type,
        other => return Err(format!("unknown VAULT_HASH_TYPE: {other}")),
    };
    let code_tx = h256(&env_req("VAULT_CODE_TX_HASH")?)?;
    let code_index: u32 = env::var("VAULT_CODE_INDEX")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .map_err(e)?;
    let code_out_point = OutPoint::new(code_tx.pack(), code_index);

    let rate_bps: u16 = env::var("RATE_BPS")
        .unwrap_or_else(|_| "250".into())
        .parse()
        .map_err(e)?;

    Ok(Cfg {
        rpc_url,
        owner_key,
        owner_lock,
        charity_locks,
        code_hash,
        hash_type,
        code_out_point,
        rate_bps,
    })
}

/// The vault lock script: your contract's code hash + the args layout from the README.
fn vault_lock(cfg: &Cfg) -> Script {
    let mut args = Vec::new();
    args.extend_from_slice(cfg.owner_lock.calc_script_hash().as_slice()); // owner_lock_hash
    args.extend_from_slice(&cfg.rate_bps.to_le_bytes()); // rate_bps
    args.push(cfg.charity_locks.len() as u8); // charity_count
    for charity_lock in &cfg.charity_locks {
        args.extend_from_slice(charity_lock.calc_script_hash().as_slice()); // charity_lock_hash[i]
    }

    Script::new_builder()
        .code_hash(cfg.code_hash.pack())
        .hash_type(cfg.hash_type)
        .args(Bytes::from(args).pack())
        .build()
}

// ---------- Helpers ----------

fn output(lock: Script, capacity: u64) -> CellOutput {
    CellOutput::new_builder()
        .capacity(capacity)
        .lock(lock)
        .build()
}

/// The default lock's code is shipped as a dep group in the genesis block.
/// This assumes the usual layout (genesis tx #1, output #0). If signing fails with a
/// missing cell dep, check your devnet's genesis and adjust.
fn sighash_dep(client: &CkbRpcClient) -> R<CellDep> {
    let genesis = client
        .get_block_by_number(0u64.into())
        .map_err(e)?
        .ok_or("genesis block not found")?;
    let tx_hash = genesis.transactions[1].hash.clone();
    Ok(CellDep::new_builder()
        .out_point(OutPoint::new(tx_hash.pack(), 0))
        .dep_type(DepType::DepGroup)
        .build())
}

fn code_dep(cfg: &Cfg) -> CellDep {
    CellDep::new_builder()
        .out_point(cfg.code_out_point.clone())
        .dep_type(DepType::Code)
        .build()
}

/// Find a plain owner cell (no type script, no data) with at least `min_capacity`.
fn pick_owner_cell(cfg: &Cfg, min_capacity: u64) -> R<(OutPoint, u64)> {
    let mut collector = DefaultCellCollector::new(&cfg.rpc_url);
    let mut query = CellQueryOptions::new_lock(cfg.owner_lock.clone());
    query.secondary_script_len_range = Some(ValueRangeOption::new_exact(0));
    query.data_len_range = Some(ValueRangeOption::new_exact(0));
    let (cells, _) = collector.collect_live_cells(&query, false).map_err(e)?;

    for cell in cells {
        let capacity: u64 = cell.output.capacity().unpack();
        if capacity >= min_capacity {
            return Ok((cell.out_point, capacity));
        }
    }
    Err("no owner cell with enough capacity (is the devnet running and funded?)".into())
}

fn placeholder_witness() -> WitnessArgs {
    WitnessArgs::new_builder()
        .lock(Some(Bytes::from(vec![0u8; 65])).pack())
        .build()
}

/// Sign the owner's input (index `input_index`) the way the default lock expects.
fn sign_input(tx: TransactionView, input_index: usize, key: &SecretKey) -> TransactionView {
    // 1. Message = blake2b(tx_hash || len(witness) || witness), with a zeroed 65-byte lock field
    let witness = placeholder_witness().as_bytes();
    let mut hasher = new_blake2b();
    hasher.update(tx.hash().as_slice());
    hasher.update(&(witness.len() as u64).to_le_bytes());
    hasher.update(&witness);
    let mut digest = [0u8; 32];
    hasher.finalize(&mut digest);

    // 2. Recoverable ECDSA signature: 64 bytes + 1 byte recovery id
    let message = Message::from_digest_slice(&digest).expect("32-byte digest");
    let (recovery_id, compact) = Secp256k1::new()
        .sign_ecdsa_recoverable(&message, key)
        .serialize_compact();
    let mut signature = compact.to_vec();
    signature.push(recovery_id.to_i32() as u8);

    // 3. Put the signature into the witness at this input's index
    let signed = WitnessArgs::new_builder()
        .lock(Some(Bytes::from(signature)).pack())
        .build();
    let mut witnesses: Vec<_> = tx.witnesses().into_iter().collect();
    witnesses[input_index] = signed.as_bytes().pack();
    tx.as_advanced_builder().set_witnesses(witnesses).build()
}

fn send(cfg: &Cfg, tx: TransactionView) -> R<()> {
    let client = CkbRpcClient::new(&cfg.rpc_url);
    // Passthrough: the default validator rejects outputs with non-standard locks (like ours)
    let hash = client
        .send_transaction(tx.data().into(), Some(OutputsValidator::Passthrough))
        .map_err(e)?;
    println!("sent: {hash:#x}");
    println!("check it with: offckb (or ckb-cli) / RPC get_transaction");
    Ok(())
}

// ---------- Commands ----------

/// Lock `ckb_amount` CKB into a new vault cell. The vault's lock does not run here (it only
/// guards spending), so this always succeeds.
fn cmd_create(cfg: &Cfg, ckb_amount: u64) -> R<()> {
    let client = CkbRpcClient::new(&cfg.rpc_url);
    let amount = ckb_amount * CKB;
    let (owner_cell, owner_cap) = pick_owner_cell(cfg, amount + MIN_CELL + FEE)?;

    let tx = TransactionBuilder::default()
        .cell_dep(sighash_dep(&client)?)
        .input(CellInput::new(owner_cell, 0))
        .output(output(vault_lock(cfg), amount))
        .output_data(Bytes::new().pack())
        .output(output(cfg.owner_lock.clone(), owner_cap - amount - FEE))
        .output_data(Bytes::new().pack())
        .witness(placeholder_witness().as_bytes().pack())
        .build();

    let tx = sign_input(tx, 0, &cfg.owner_key);
    println!(
        "vault created at output index 0 of the tx below ({} approved charit{})",
        cfg.charity_locks.len(),
        if cfg.charity_locks.len() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    send(cfg, tx)
}

/// Parses a `--split a,b,...` value (CKB amounts) into per-charity shannon payouts, aligned
/// with `cfg.charity_locks` by index. Missing trailing entries default to 0.
fn parse_split(cfg: &Cfg, spec: &str) -> R<Vec<u64>> {
    let mut amounts: Vec<u64> = Vec::new();
    for part in spec.split(',') {
        let ckb: u64 = part.trim().parse().map_err(e)?;
        amounts.push(ckb * CKB);
    }
    if amounts.len() > cfg.charity_locks.len() {
        return Err(format!(
            "--split lists {} amounts but only {} charities are approved",
            amounts.len(),
            cfg.charity_locks.len()
        ));
    }
    amounts.resize(cfg.charity_locks.len(), 0);
    Ok(amounts)
}

/// Withdraw `ckb_amount` from the vault cell, paying the charity share.
///
/// - By default, the full required share goes to the first approved charity.
/// - With `short`, pay only 70% of the required share, which the contract should reject.
/// - With `split`, pay the exact per-charity amounts given (CKB), overriding the above.
fn cmd_withdraw(
    cfg: &Cfg,
    vault_tx: &str,
    vault_index: u32,
    ckb_amount: u64,
    short: bool,
    split: Option<&str>,
) -> R<()> {
    if short && split.is_some() {
        return Err("--short and --split are mutually exclusive".into());
    }

    let client = CkbRpcClient::new(&cfg.rpc_url);
    let vault_out_point = OutPoint::new(h256(vault_tx)?.pack(), vault_index);

    // Read the vault cell's current capacity
    let live = client
        .get_live_cell(vault_out_point.clone().into(), false)
        .map_err(e)?;
    let vault_output = CellOutput::from(
        live.cell
            .ok_or("vault cell not live (already spent?)")?
            .output,
    );
    let vault_cap: u64 = vault_output.capacity().unpack();

    let withdrawn = ckb_amount * CKB;
    if withdrawn > vault_cap {
        return Err("withdrawal is larger than the vault".into());
    }

    // required = ceil(withdrawn * rate_bps / 10000), the same rule as the contract
    let required = ((withdrawn as u128 * cfg.rate_bps as u128 + 9_999) / 10_000) as u64;

    let payouts: Vec<u64> = if let Some(spec) = split {
        parse_split(cfg, spec)?
    } else {
        let amount = if short {
            required * 70 / 100
        } else {
            required.max(MIN_CELL)
        };
        let mut v = vec![0u64; cfg.charity_locks.len()];
        v[0] = amount;
        v
    };
    let total_paid: u64 = payouts.iter().sum();
    for (i, &amount) in payouts.iter().enumerate() {
        if amount > 0 && amount < MIN_CELL {
            return Err(format!(
                "payout to charity #{i} ({amount} shannons) is below the minimum cell capacity ({MIN_CELL}); \
                 pay 0 or at least {MIN_CELL} shannons to that charity"
            ));
        }
    }
    println!(
        "withdrawn {} CKB, required share {} shannons, paying {} shannons across {} charit{}",
        ckb_amount,
        required,
        total_paid,
        payouts.iter().filter(|&&a| a > 0).count(),
        if payouts.iter().filter(|&&a| a > 0).count() == 1 {
            "y"
        } else {
            "ies"
        }
    );

    // The owner's input is what proves authorization, and it also pays the fee
    let (owner_cell, owner_cap) = pick_owner_cell(cfg, MIN_CELL + FEE)?;
    let vault_remaining = vault_cap - withdrawn;
    let owner_change = (vault_cap + owner_cap)
        .checked_sub(vault_remaining + total_paid + FEE)
        .ok_or("not enough capacity")?;
    if owner_change < MIN_CELL {
        return Err("owner change would be below the minimum cell size".into());
    }

    let mut builder = TransactionBuilder::default()
        .cell_dep(code_dep(cfg))
        .cell_dep(sighash_dep(&client)?)
        .input(CellInput::new(vault_out_point, 0)) // index 0: the vault
        .input(CellInput::new(owner_cell, 0)) // index 1: the owner
        .output(output(vault_lock(cfg), vault_remaining)) // stays in the vault
        .output_data(Bytes::new().pack());
    for (charity_lock, &amount) in cfg.charity_locks.iter().zip(payouts.iter()) {
        if amount > 0 {
            builder = builder
                .output(output(charity_lock.clone(), amount))
                .output_data(Bytes::new().pack());
        }
    }
    let tx = builder
        .output(output(cfg.owner_lock.clone(), owner_change)) // the rest back to the owner
        .output_data(Bytes::new().pack())
        .witness(Bytes::new().pack()) // the vault lock needs no witness
        .witness(placeholder_witness().as_bytes().pack())
        .build();

    let tx = sign_input(tx, 1, &cfg.owner_key);
    send(cfg, tx)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let result = (|| -> R<()> {
        let cfg = load_cfg()?;
        match args.get(1).map(String::as_str) {
            Some("create") => {
                let amount = args.get(2).ok_or("usage: create <ckb_amount>")?.parse().map_err(e)?;
                cmd_create(&cfg, amount)
            }
            Some("withdraw") => {
                let tx = args
                    .get(2)
                    .ok_or("usage: withdraw <vault_tx_hash> <index> <ckb_amount> [--short | --split a,b,...]")?;
                let index = args.get(3).ok_or("missing vault index")?.parse().map_err(e)?;
                let amount = args.get(4).ok_or("missing ckb amount")?.parse().map_err(e)?;
                let short = args.iter().any(|a| a == "--short");
                let split = args
                    .iter()
                    .position(|a| a == "--split")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str);
                cmd_withdraw(&cfg, tx, index, amount, short, split)
            }
            _ => Err(
                "commands: create <ckb_amount> | withdraw <vault_tx_hash> <index> <ckb_amount> [--short | --split a,b,...]"
                    .into(),
            ),
        }
    })();

    if let Err(msg) = result {
        eprintln!("error: {msg}");
        std::process::exit(1);
    }
}
