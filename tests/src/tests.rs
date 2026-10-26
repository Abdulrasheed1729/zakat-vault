use crate::Loader;
use ckb_testtool::builtin::ALWAYS_SUCCESS;
use ckb_testtool::ckb_types::core::ScriptHashType;
use ckb_testtool::ckb_types::{bytes::Bytes, core::TransactionBuilder, packed::*, prelude::*};
use ckb_testtool::context::Context;

const MAX_CYCLES: u64 = 70_000_000;
const CKB: u64 = 100_000_000; // 1 CKB in shannons
const RATE_BPS: u16 = 250; // 2.5%

// Error codes mirror contracts/sadaqah-lock/src/error.rs (#[repr(i8)] enum Error)
const ERR_LENGTH_NOT_ENOUGH: i8 = 3;
const ERR_OWNER_NOT_AUTHORIZED: i8 = 7;
const ERR_CHARITY_SHORT_PAID: i8 = 8;
const ERR_BAD_CHARITY_COUNT: i8 = 9;
const ERR_DUPLICATE_CHARITY: i8 = 10;

/// Encodes vault args: owner_hash(32) | rate_bps(2, LE) | charity_count(1) | charity_hash[1..=N](32 each)
fn encode_args(owner_hash: &[u8], rate_bps: u16, charity_hashes: &[Vec<u8>]) -> Vec<u8> {
    let mut args = Vec::new();
    args.extend_from_slice(owner_hash);
    args.extend_from_slice(&rate_bps.to_le_bytes());
    args.push(charity_hashes.len() as u8);
    for hash in charity_hashes {
        args.extend_from_slice(hash);
    }
    args
}

/// Parameters for a single vault transaction test case. Use `..Default::default()`
/// to only override the fields relevant to the scenario under test.
struct CaseParams {
    /// Capacity (in CKB) entering the vault as an input.
    vault_in: u64,
    /// Capacity (in CKB) returned to the vault as an output.
    vault_out: u64,
    /// Number of approved charity locks to generate and encode into the vault args.
    num_charities: u8,
    /// Capacity (in CKB) paid to each approved charity, indexed the same as the
    /// generated charity locks (0 means no output is created for that charity).
    charity_payouts: Vec<u64>,
    /// Whether an owner-locked input is present in the transaction.
    include_owner: bool,
    /// When set, replaces the normally encoded args (used to test malformed args).
    args_override: Option<Vec<u8>>,
    /// When true, every approved charity slot is filled with the same hash
    /// (used to test duplicate-charity rejection).
    duplicate_charities: bool,
    /// Capacity (in CKB) paid to an arbitrary lock that is NOT in the approved
    /// charity list (used to verify such payments don't count toward the share).
    non_approved_payout: u64,
}

impl Default for CaseParams {
    fn default() -> Self {
        Self {
            vault_in: 2000,
            vault_out: 1000,
            num_charities: 1,
            charity_payouts: vec![25],
            include_owner: true,
            args_override: None,
            duplicate_charities: false,
            non_approved_payout: 0,
        }
    }
}

/// Builds a transaction per `params` and verifies it.
fn run_case(params: CaseParams) -> Result<u64, ckb_testtool::ckb_error::Error> {
    let mut context = Context::default();

    // Deploy the contract under test and an always-success lock
    // (the always-success lock stands in for the owner's/charity's real wallet lock)
    let vault_bin: Bytes = Loader::default().load_binary("sadaqah-lock");
    let vault_op = context.deploy_cell(vault_bin);
    let always_op = context.deploy_cell(ALWAYS_SUCCESS.clone());

    let owner_lock = context
        .build_script_with_hash_type(&always_op, ScriptHashType::Data1, Bytes::from(vec![1]))
        .unwrap();

    let charity_locks: Vec<Script> = (0..params.num_charities)
        .map(|i| {
            context
                .build_script_with_hash_type(
                    &always_op,
                    ScriptHashType::Data1,
                    Bytes::from(vec![10 + i]),
                )
                .unwrap()
        })
        .collect();

    let mut charity_hashes: Vec<Vec<u8>> = charity_locks
        .iter()
        .map(|l| l.calc_script_hash().as_slice().to_vec())
        .collect();
    if params.duplicate_charities {
        if let Some(first) = charity_hashes.first().cloned() {
            charity_hashes = charity_hashes.iter().map(|_| first.clone()).collect();
        }
    }

    let args = params.args_override.unwrap_or_else(|| {
        encode_args(
            owner_lock.calc_script_hash().as_slice(),
            RATE_BPS,
            &charity_hashes,
        )
    });

    let vault_lock = context
        .build_script_with_hash_type(&vault_op, ScriptHashType::Data1, Bytes::from(args))
        .unwrap();

    // Helper to create a live input cell
    let mut make_input = |lock: &Script, capacity_ckb: u64| {
        let output = CellOutput::new_builder()
            .capacity(capacity_ckb * CKB)
            .lock(lock.clone())
            .build();
        let op = context.create_cell(output, Bytes::new());
        CellInput::new_builder().previous_output(op).build()
    };

    // Inputs
    let mut inputs = vec![make_input(&vault_lock, params.vault_in)];
    if params.include_owner {
        inputs.push(make_input(&owner_lock, 100));
    }

    // Outputs
    let make_output = |lock: &Script, capacity_ckb: u64| {
        CellOutput::new_builder()
            .capacity(capacity_ckb * CKB)
            .lock(lock.clone())
            .build()
    };
    let mut outputs = Vec::new();
    if params.vault_out > 0 {
        outputs.push(make_output(&vault_lock, params.vault_out));
    }
    for (i, &amount) in params.charity_payouts.iter().enumerate() {
        if amount > 0 {
            outputs.push(make_output(&charity_locks[i], amount));
        }
    }
    if params.non_approved_payout > 0 {
        let unapproved_lock = context
            .build_script_with_hash_type(&always_op, ScriptHashType::Data1, Bytes::from(vec![99]))
            .unwrap();
        outputs.push(make_output(&unapproved_lock, params.non_approved_payout));
    }
    // The rest goes back to the owner (balance doesn't matter for script verification,
    // but we keep the numbers realistic)
    outputs.push(make_output(&owner_lock, 100));

    let outputs_data: Vec<_> = outputs.iter().map(|_| Bytes::new().pack()).collect();

    let tx = TransactionBuilder::default()
        .inputs(inputs)
        .outputs(outputs)
        .outputs_data(outputs_data)
        .build();
    let tx = context.complete_tx(tx);

    context.verify_tx(&tx, MAX_CYCLES)
}

fn assert_error_code(result: Result<u64, ckb_testtool::ckb_error::Error>, code: i8) {
    let err = result.expect_err("expected the script to fail");
    let msg = err.to_string();
    assert!(
        msg.contains(&format!("error code {}", code)),
        "expected error code {}, got: {}",
        code,
        msg
    );
}

#[test]
fn test_withdraw_pays_correct_charity_share() {
    // Withdraw 1000 CKB from the vault, charity gets exactly 2.5% (25 CKB)
    let cycles = run_case(CaseParams::default()).expect("pass verification");
    println!("consume cycles: {}", cycles);
}

#[test]
fn test_withdraw_overpays_charity_is_allowed() {
    // Paying more than the required share is fine
    run_case(CaseParams {
        charity_payouts: vec![30],
        ..Default::default()
    })
    .expect("pass verification");
}

#[test]
fn test_topup_without_withdrawal_needs_no_charity_payment() {
    // vault_out >= vault_in: nothing left the vault, so no charity payment is required
    run_case(CaseParams {
        vault_in: 1000,
        vault_out: 1500,
        charity_payouts: vec![0],
        ..Default::default()
    })
    .expect("pass verification");
}

#[test]
fn test_withdraw_without_charity_payment_fails() {
    let result = run_case(CaseParams {
        charity_payouts: vec![0],
        ..Default::default()
    });
    assert_error_code(result, ERR_CHARITY_SHORT_PAID);
}

#[test]
fn test_withdraw_underpaying_charity_fails() {
    // 1000 CKB withdrawn requires 25 CKB (2.5%); pay 24 instead
    let result = run_case(CaseParams {
        charity_payouts: vec![24],
        ..Default::default()
    });
    assert_error_code(result, ERR_CHARITY_SHORT_PAID);
}

#[test]
fn test_withdraw_without_owner_input_fails() {
    let result = run_case(CaseParams {
        include_owner: false,
        ..Default::default()
    });
    assert_error_code(result, ERR_OWNER_NOT_AUTHORIZED);
}

#[test]
fn test_bad_args_length_fails() {
    // Too short to even contain the fixed header (owner + rate + charity_count)
    let result = run_case(CaseParams {
        args_override: Some(vec![0u8; 10]),
        ..Default::default()
    });
    assert_error_code(result, ERR_LENGTH_NOT_ENOUGH);
}

#[test]
fn test_withdraw_split_across_two_charities_passes() {
    // 1000 CKB withdrawn requires 25 CKB total; split 15 + 10 across both approved charities
    run_case(CaseParams {
        num_charities: 2,
        charity_payouts: vec![15, 10],
        ..Default::default()
    })
    .expect("pass verification");
}

#[test]
fn test_payment_to_unapproved_charity_does_not_count() {
    // The full 25 CKB goes to a lock that isn't in the approved charity list
    let result = run_case(CaseParams {
        charity_payouts: vec![0],
        non_approved_payout: 25,
        ..Default::default()
    });
    assert_error_code(result, ERR_CHARITY_SHORT_PAID);
}

#[test]
fn test_duplicate_charity_in_args_fails() {
    let result = run_case(CaseParams {
        num_charities: 2,
        charity_payouts: vec![15, 10],
        duplicate_charities: true,
        ..Default::default()
    });
    assert_error_code(result, ERR_DUPLICATE_CHARITY);
}

#[test]
fn test_zero_charities_fails() {
    let result = run_case(CaseParams {
        num_charities: 0,
        charity_payouts: vec![],
        ..Default::default()
    });
    assert_error_code(result, ERR_BAD_CHARITY_COUNT);
}

#[test]
fn test_too_many_charities_fails() {
    let result = run_case(CaseParams {
        num_charities: 5,
        charity_payouts: vec![5, 5, 5, 5, 5],
        ..Default::default()
    });
    assert_error_code(result, ERR_BAD_CHARITY_COUNT);
}
