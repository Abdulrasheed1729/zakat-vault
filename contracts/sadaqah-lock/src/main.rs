#![cfg_attr(not(any(feature = "library", test)), no_std)]
#![cfg_attr(not(test), no_main)]

mod error;

// `ckb_std::entry!` already declares `extern crate alloc;` for the real on-chain
// build path, so we only need to declare it ourselves for the library/test paths.
#[cfg(any(feature = "library", test))]
extern crate alloc;

#[cfg(not(any(feature = "library", test)))]
ckb_std::entry!(program_entry);
#[cfg(not(any(feature = "library", test)))]
// By default, the following heap configuration is used:
// * 16KB fixed heap
// * 1.2MB(rounded up to be 16-byte aligned) dynamic heap
// * Minimal memory block in dynamic heap is 64 bytes
// For more details, please refer to ckb-std's default_alloc macro
// and the buddy-alloc alloc implementation.
ckb_std::default_alloc!(16384, 1258306, 64);

use alloc::vec::Vec;
use ckb_std::ckb_constants::Source;
use ckb_std::error::SysError;
use ckb_std::high_level::{
    QueryIter, load_cell_capacity, load_cell_lock_hash, load_script, load_script_hash,
};
use error::Error;

// args layout:
//   owner_lock_hash       : 32 bytes
//   rate_bps              : 2  bytes (u16, little-endian)
//   charity_count (N)     : 1  byte  (1..=MAX_CHARITIES)
//   charity_lock_hash[1]  : 32 bytes
//   ...
//   charity_lock_hash[N]  : 32 bytes
const HEADER_LEN: usize = 32 + 2 + 1;
const CHARITY_HASH_LEN: usize = 32;
const MIN_CHARITIES: usize = 1;
const MAX_CHARITIES: usize = 4;
const BPS_DENOMINATOR: u128 = 10_000;

struct Args {
    owner_lock_hash: [u8; 32],
    rate_bps: u16,
    charity_lock_hashes: Vec<[u8; 32]>,
}

fn parse_args() -> Result<Args, Error> {
    let script = load_script()?;
    let raw = script.args().raw_data();

    if raw.len() < HEADER_LEN {
        return Err(Error::LengthNotEnough);
    }

    let mut owner_lock_hash = [0u8; 32];
    owner_lock_hash.copy_from_slice(&raw[0..32]);
    let rate_bps = u16::from_le_bytes([raw[32], raw[33]]);
    let charity_count = raw[34] as usize;

    if !(MIN_CHARITIES..=MAX_CHARITIES).contains(&charity_count) {
        return Err(Error::BadCharityCount);
    }

    let expected_len = HEADER_LEN + charity_count * CHARITY_HASH_LEN;
    if raw.len() != expected_len {
        return Err(Error::BadCharityCount);
    }

    if rate_bps == 0 || rate_bps as u128 > BPS_DENOMINATOR {
        return Err(Error::BadRate);
    }

    let mut charity_lock_hashes: Vec<[u8; 32]> = Vec::with_capacity(charity_count);
    for i in 0..charity_count {
        let start = HEADER_LEN + i * CHARITY_HASH_LEN;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&raw[start..start + CHARITY_HASH_LEN]);
        if charity_lock_hashes.contains(&hash) {
            return Err(Error::DuplicateCharity);
        }
        charity_lock_hashes.push(hash);
    }

    Ok(Args {
        owner_lock_hash,
        rate_bps,
        charity_lock_hashes,
    })
}

/// Sum the capacity of all cells in `source` whose lock hash equals `lock_hash`.
fn sum_capacity_with_lock(source: Source, lock_hash: &[u8; 32]) -> Result<u128, Error> {
    let mut total: u128 = 0;
    let mut index = 0;
    loop {
        match load_cell_lock_hash(index, source) {
            Ok(hash) => {
                if &hash == lock_hash {
                    total += load_cell_capacity(index, source)? as u128;
                }
            }
            Err(SysError::IndexOutOfBound) => break,
            Err(e) => return Err(e.into()),
        }
        index += 1;
    }
    Ok(total)
}

fn check_owner(args: &Args) -> Result<(), Error> {
    let owner_present =
        QueryIter::new(load_cell_lock_hash, Source::Input).any(|hash| hash == args.owner_lock_hash);
    if owner_present {
        Ok(())
    } else {
        Err(Error::OwnerNotAuthorized)
    }
}

fn compute_withdrawn() -> Result<u128, Error> {
    let my_lock_hash = load_script_hash()?;
    let vault_in = sum_capacity_with_lock(Source::Input, &my_lock_hash)?;
    let vault_out = sum_capacity_with_lock(Source::Output, &my_lock_hash)?;
    // If nothing left the vault (e.g. a top-up), withdrawn is 0
    Ok(vault_in.saturating_sub(vault_out))
}

/// Sum of outputs paid to any of the approved charity locks. The owner may
/// split a single withdrawal's required share across any subset of the
/// approved charities.
fn sum_charity_payouts(args: &Args) -> Result<u128, Error> {
    let mut paid: u128 = 0;
    for charity_hash in &args.charity_lock_hashes {
        paid += sum_capacity_with_lock(Source::Output, charity_hash)?;
    }
    Ok(paid)
}

fn check_charity_share(args: &Args, withdrawn: u128) -> Result<(), Error> {
    if withdrawn == 0 {
        return Ok(());
    }

    let numerator = withdrawn
        .checked_mul(args.rate_bps as u128)
        .ok_or(Error::MathOverflow)?;
    // Round up so the charity is never short-changed
    let required = (numerator + BPS_DENOMINATOR - 1) / BPS_DENOMINATOR;

    let paid = sum_charity_payouts(args)?;

    #[cfg(feature = "enable_log")]
    ckb_std::debug!(
        "sadaqah-lock: withdrawn={} required={} paid={}",
        withdrawn,
        required,
        paid
    );

    if paid >= required {
        Ok(())
    } else {
        Err(Error::CharityShortPaid)
    }
}

fn run() -> Result<(), Error> {
    let args = parse_args()?;
    check_owner(&args)?;
    let withdrawn = compute_withdrawn()?;
    check_charity_share(&args, withdrawn)?;
    Ok(())
}

fn program_entry() -> i8 {
    match run() {
        Ok(()) => 0,
        Err(e) => e as i8,
    }
}
