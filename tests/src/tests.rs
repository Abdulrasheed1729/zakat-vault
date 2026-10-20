use ckb_testtool::builtin::ALWAYS_SUCCESS;
use ckb_testtool::ckb_types::{bytes::Bytes, core::TransactionBuilder, packed::*, prelude::*};
use ckb_testtool::ckb_types::core::ScriptHashType;
use ckb_testtool::context::Context;
use crate::Loader;
// Include your tests here
// See https://github.com/xxuejie/ckb-native-build-sample/blob/main/tests/src/tests.rs for more examples

const MAX_CYCLES: u64 = 70_000_000;
const CKB: u64 = 100_000_000; // 1 CKB in shannons
const RATE_BPS: u16 = 250; // 2.5%

/// Builds a transaction and verifies it.
/// - vault_in / vault_out: capacity (in CKB) entering and returning to the vault
/// - charity_out: capacity (in CKB) paid to the charity lock
/// - include_owner: whether an owner-locked input is present
/// - args_len: Some(n) truncates the vault args to n bytes (to test bad args)
fn run_case(
    vault_in: u64,
    vault_out: u64,
    charity_out: u64,
    include_owner: bool,
    args_len: Option<usize>,
) -> Result<u64, ckb_testtool::ckb_error::Error> {
    let mut context = Context::default();

    // Deploy the contract under test and an always-success lock
    // (the always-success lock stands in for the owner's/charity's real wallet lock)
    let vault_bin: Bytes = Loader::default().load_binary("zakat-lock");
    let vault_op = context.deploy_cell(vault_bin);
    let always_op = context.deploy_cell(ALWAYS_SUCCESS.clone());

    let owner_lock = context
        .build_script_with_hash_type(&always_op, ScriptHashType::Data1, Bytes::from(vec![1]))
        .unwrap();
    let charity_lock = context
        .build_script_with_hash_type(&always_op, ScriptHashType::Data1, Bytes::from(vec![2]))
        .unwrap();

    // Vault args: owner_hash (32) + charity_hash (32) + rate_bps (2, little endian)
    let mut args = Vec::new();
    args.extend_from_slice(owner_lock.calc_script_hash().as_slice());
    args.extend_from_slice(charity_lock.calc_script_hash().as_slice());
    args.extend_from_slice(&RATE_BPS.to_le_bytes());
    if let Some(n) = args_len {
        args.truncate(n);
    }

    let vault_lock = context
        .build_script_with_hash_type(&vault_op, ScriptHashType::Data1, Bytes::from(args))
        .unwrap();

    // Helper to create a live input cell
    let mut make_input = |lock: &Script, capacity_ckb: u64| {
        let output = CellOutput::new_builder()
            .capacity((capacity_ckb * CKB).pack())
            .lock(lock.clone())
            .build();
        let op = context.create_cell(output, Bytes::new());
        CellInput::new_builder().previous_output(op).build()
    };

    // Inputs
    let mut inputs = vec![make_input(&vault_lock, vault_in)];
    if include_owner {
        inputs.push(make_input(&owner_lock, 100));
    }

    // Outputs
    let make_output = |lock: &Script, capacity_ckb: u64| {
        CellOutput::new_builder()
            .capacity((capacity_ckb * CKB).pack())
            .lock(lock.clone())
            .build()
    };
    let mut outputs = Vec::new();
    if vault_out > 0 {
        outputs.push(make_output(&vault_lock, vault_out));
    }
    if charity_out > 0 {
        outputs.push(make_output(&charity_lock, charity_out));
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

// generated unit test for contract zakat-lock
#[test]
fn test_sadaqah_lock() {
    // deploy contract
    let mut context = Context::default();
    let out_point = context.deploy_cell_by_name("sadaqah-lock");

    // prepare scripts
    let lock_script = context
        .build_script(&out_point, Bytes::from(vec![42]))
        .expect("script");

    // prepare cells
    let input_out_point = context.create_cell(
        CellOutput::new_builder()
            .capacity(1000)
            .lock(lock_script.clone())
            .build(),
        Bytes::new(),
    );
    let input = CellInput::new_builder()
        .previous_output(input_out_point)
        .build();
    let outputs = vec![
        CellOutput::new_builder()
            .capacity(500)
            .lock(lock_script.clone())
            .build(),
        CellOutput::new_builder()
            .capacity(500)
            .lock(lock_script)
            .build(),
    ];

    let outputs_data = vec![Bytes::new(); 2];

    // build transaction
    let tx = TransactionBuilder::default()
        .input(input)
        .outputs(outputs)
        .outputs_data(outputs_data.pack())
        .build();
    let tx = context.complete_tx(tx);

    // run
    let cycles = context
        .verify_tx(&tx, 10_000_000)
        .expect("pass verification");
    println!("consume cycles: {}", cycles);
}
