// LiteSVM tests. loads target/deploy/amm.so and calls each instruction.
// run: anchor build --no-idl && cd tests && cargo test

use litesvm::LiteSVM;
use solana_sdk::{
    hash::hash,
    instruction::{AccountMeta, Instruction},
    native_token::LAMPORTS_PER_SOL,
    program_pack::Pack,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::Transaction,
};
use spl_associated_token_account::get_associated_token_address;
use spl_token::state::{Account as SplTokenAccount, Mint};
use std::str::FromStr;

const PROGRAM_ID: &str = "8tBhBep6XngjnJD3qvXCQwV9ToxMvFebgm4qpDTDhy4u";
const SO_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../target/deploy/amm.so");

// harness

struct Env {
    svm: LiteSVM,
    program_id: Pubkey,
    payer: Keypair,
}

fn setup() -> Env {
    let mut svm = LiteSVM::new();
    let program_id = Pubkey::from_str(PROGRAM_ID).unwrap();
    svm.add_program_from_file(program_id, SO_PATH)
        .expect("failed to load amm.so — run `anchor build --no-idl` first");

    let payer = Keypair::new();
    svm.airdrop(&payer.pubkey(), 100 * LAMPORTS_PER_SOL).unwrap();

    Env { svm, program_id, payer }
}

// sign and send
fn send(
    svm: &mut LiteSVM,
    ixs: &[Instruction],
    payer: &Keypair,
    extra_signers: &[&Keypair],
) -> Result<(), String> {
    let mut signers = vec![payer];
    signers.extend_from_slice(extra_signers);
    let bh = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(ixs, Some(&payer.pubkey()), &signers, bh);
    svm.send_transaction(tx)
        .map(|_| ())
        .map_err(|e| format!("{:?}\nlogs:\n{}", e.err, e.meta.logs.join("\n")))
}

// token helpers

fn create_mint(env: &mut Env, decimals: u8) -> Pubkey {
    let mint = Keypair::new();
    let rent = env.svm.minimum_balance_for_rent_exemption(Mint::LEN);
    let payer_pk = env.payer.pubkey();
    let ixs = [
        system_instruction::create_account(
            &payer_pk,
            &mint.pubkey(),
            rent,
            Mint::LEN as u64,
            &spl_token::ID,
        ),
        spl_token::instruction::initialize_mint2(
            &spl_token::ID,
            &mint.pubkey(),
            &payer_pk, // mint authority = payer
            None,
            decimals,
        )
        .unwrap(),
    ];
    let payer = env.payer.insecure_clone();
    send(&mut env.svm, &ixs, &payer, &[&mint]).unwrap();
    mint.pubkey()
}

fn create_ata(env: &mut Env, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    let payer = env.payer.insecure_clone();
    let ix = spl_associated_token_account::instruction::create_associated_token_account(
        &payer.pubkey(),
        owner,
        mint,
        &spl_token::ID,
    );
    send(&mut env.svm, &[ix], &payer, &[]).unwrap();
    get_associated_token_address(owner, mint)
}

fn mint_to(env: &mut Env, mint: &Pubkey, dest: &Pubkey, amount: u64) {
    let payer = env.payer.insecure_clone();
    let ix = spl_token::instruction::mint_to(
        &spl_token::ID,
        mint,
        dest,
        &payer.pubkey(),
        &[],
        amount,
    )
    .unwrap();
    send(&mut env.svm, &[ix], &payer, &[]).unwrap();
}

fn token_balance(env: &Env, ata: &Pubkey) -> u64 {
    let acc = env.svm.get_account(ata).expect("token account missing");
    SplTokenAccount::unpack(&acc.data).unwrap().amount
}

// pda + instruction builders

fn ix_disc(name: &str) -> [u8; 8] {
    let h = hash(format!("global:{}", name).as_bytes());
    h.to_bytes()[..8].try_into().unwrap()
}

fn config_pda(program_id: &Pubkey, seed: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config", &seed.to_le_bytes()], program_id)
}

fn lp_pda(program_id: &Pubkey, config: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"lp", config.as_ref()], program_id)
}

struct Pool {
    seed: u64,
    config: Pubkey,
    config_bump: u8,
    mint_lp: Pubkey,
    lp_bump: u8,
    mint_x: Pubkey,
    mint_y: Pubkey,
    vault_x: Pubkey,
    vault_y: Pubkey,
}

fn pool_for(program_id: &Pubkey, seed: u64, mint_x: Pubkey, mint_y: Pubkey) -> Pool {
    let (config, config_bump) = config_pda(program_id, seed);
    let (mint_lp, lp_bump) = lp_pda(program_id, &config);
    let vault_x = get_associated_token_address(&config, &mint_x);
    let vault_y = get_associated_token_address(&config, &mint_y);
    Pool { seed, config, config_bump, mint_lp, lp_bump, mint_x, mint_y, vault_x, vault_y }
}

fn initialize_ix(env: &Env, pool: &Pool, fee: u16, authority: Option<Pubkey>) -> Instruction {
    let mut data = ix_disc("initialize").to_vec();
    data.extend_from_slice(&pool.seed.to_le_bytes());
    data.extend_from_slice(&fee.to_le_bytes());
    match authority {
        Some(pk) => {
            data.push(1);
            data.extend_from_slice(pk.as_ref());
        }
        None => data.push(0),
    }

    let accounts = vec![
        AccountMeta::new(env.payer.pubkey(), true),
        AccountMeta::new_readonly(pool.mint_x, false),
        AccountMeta::new_readonly(pool.mint_y, false),
        AccountMeta::new(pool.config, false),
        AccountMeta::new(pool.mint_lp, false),
        AccountMeta::new(pool.vault_x, false),
        AccountMeta::new(pool.vault_y, false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
    ];
    Instruction { program_id: env.program_id, accounts, data }
}

fn deposit_ix(
    env: &Env,
    pool: &Pool,
    user: &Pubkey,
    amount: u64,
    max_x: u64,
    max_y: u64,
) -> Instruction {
    let mut data = ix_disc("deposit").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&max_x.to_le_bytes());
    data.extend_from_slice(&max_y.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(pool.config, false),
        AccountMeta::new_readonly(pool.mint_x, false),
        AccountMeta::new_readonly(pool.mint_y, false),
        AccountMeta::new(pool.mint_lp, false),
        AccountMeta::new(pool.vault_x, false),
        AccountMeta::new(pool.vault_y, false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_x), false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_y), false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_lp), false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
    ];
    Instruction { program_id: env.program_id, accounts, data }
}

fn swap_ix(
    env: &Env,
    pool: &Pool,
    user: &Pubkey,
    is_x: bool,
    amount_in: u64,
    min_out: u64,
) -> Instruction {
    let mut data = ix_disc("swap").to_vec();
    data.push(is_x as u8);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(pool.config, false),
        AccountMeta::new_readonly(pool.mint_x, false),
        AccountMeta::new_readonly(pool.mint_y, false),
        AccountMeta::new(pool.vault_x, false),
        AccountMeta::new(pool.vault_y, false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_x), false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_y), false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
    ];
    Instruction { program_id: env.program_id, accounts, data }
}

fn withdraw_ix(
    env: &Env,
    pool: &Pool,
    user: &Pubkey,
    amount: u64,
    min_x: u64,
    min_y: u64,
) -> Instruction {
    let mut data = ix_disc("withdraw").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&min_x.to_le_bytes());
    data.extend_from_slice(&min_y.to_le_bytes());

    let accounts = vec![
        AccountMeta::new(*user, true),
        AccountMeta::new_readonly(pool.config, false),
        AccountMeta::new_readonly(pool.mint_x, false),
        AccountMeta::new_readonly(pool.mint_y, false),
        AccountMeta::new(pool.mint_lp, false),
        AccountMeta::new(pool.vault_x, false),
        AccountMeta::new(pool.vault_y, false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_x), false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_y), false),
        AccountMeta::new(get_associated_token_address(user, &pool.mint_lp), false),
        AccountMeta::new_readonly(spl_token::ID, false),
        AccountMeta::new_readonly(spl_associated_token_account::ID, false),
        AccountMeta::new_readonly(solana_sdk::system_program::ID, false),
    ];
    Instruction { program_id: env.program_id, accounts, data }
}

// same math as the program, to compute expected values

fn expected_out(amount_in: u64, reserve_in: u64, reserve_out: u64, fee_bps: u16) -> u64 {
    let amount_in = amount_in as u128;
    let in_after_fee = amount_in * (10_000 - fee_bps as u128) / 10_000;
    let out = (reserve_out as u128 * in_after_fee) / (reserve_in as u128 + in_after_fee);
    out as u64
}

fn expected_deposit_ceil(amount: u64, reserve: u64, supply: u64) -> u64 {
    let n = amount as u128 * reserve as u128;
    ((n + supply as u128 - 1) / supply as u128) as u64
}

fn expected_withdraw_floor(amount: u64, reserve: u64, supply: u64) -> u64 {
    (amount as u128 * reserve as u128 / supply as u128) as u64
}

// funded user + a pool with liquidity

struct Scenario {
    pool: Pool,
    user: Keypair,
    user_x: Pubkey,
    user_y: Pubkey,
    user_lp: Pubkey,
    fee_bps: u16,
}

// init a pool, seed it, and mint LP to the user (with spare balance left over)
fn liquid_pool(env: &mut Env, fee_bps: u16, init_x: u64, init_y: u64, lp_amount: u64) -> Scenario {
    let mint_x = create_mint(env, 6);
    let mint_y = create_mint(env, 6);
    let seed = 42;
    let pool = pool_for(&env.program_id, seed, mint_x, mint_y);

    let ix = initialize_ix(env, &pool, fee_bps, None);
    let payer = env.payer.insecure_clone();
    send(&mut env.svm, &[ix], &payer, &[]).unwrap();

    // fund the user with 10x the seed amount of each token
    let user = Keypair::new();
    env.svm.airdrop(&user.pubkey(), 10 * LAMPORTS_PER_SOL).unwrap();
    let user_x = create_ata(env, &user.pubkey(), &mint_x);
    let user_y = create_ata(env, &user.pubkey(), &mint_y);
    let user_lp = get_associated_token_address(&user.pubkey(), &pool.mint_lp);
    mint_to(env, &mint_x, &user_x, init_x * 10);
    mint_to(env, &mint_y, &user_y, init_y * 10);

    // seed liquidity
    let ix = deposit_ix(env, &pool, &user.pubkey(), lp_amount, init_x, init_y);
    send(&mut env.svm, &[ix], &user, &[]).unwrap();

    Scenario { pool, user, user_x, user_y, user_lp, fee_bps }
}

// happy paths

#[test]
fn test_initialize() {
    let mut env = setup();
    let mint_x = create_mint(&mut env, 6);
    let mint_y = create_mint(&mut env, 6);
    let pool = pool_for(&env.program_id, 1, mint_x, mint_y);

    let ix = initialize_ix(&env, &pool, 30, None);
    let payer = env.payer.insecure_clone();
    send(&mut env.svm, &[ix], &payer, &[]).unwrap();

    let acc = env.svm.get_account(&pool.config).expect("config not created");
    let d = &acc.data;
    assert_eq!(acc.owner, env.program_id);

    // layout: disc(8) seed(8) auth_tag(1) mint_x(32) mint_y(32) fee(2) locked(1) bumps(2)
    let seed = u64::from_le_bytes(d[8..16].try_into().unwrap());
    assert_eq!(seed, 1);
    assert_eq!(d[16], 0, "authority should be None");
    assert_eq!(&d[17..49], mint_x.as_ref());
    assert_eq!(&d[49..81], mint_y.as_ref());
    let fee = u16::from_le_bytes(d[81..83].try_into().unwrap());
    assert_eq!(fee, 30);
    assert_eq!(d[83], 0, "pool should start unlocked");
    assert_eq!(d[84], pool.config_bump);
    assert_eq!(d[85], pool.lp_bump);

    assert!(env.svm.get_account(&pool.mint_lp).is_some());
    assert_eq!(token_balance(&env, &pool.vault_x), 0);
    assert_eq!(token_balance(&env, &pool.vault_y), 0);
}

#[test]
fn test_deposit_initial_liquidity() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    // first deposit = exactly the caps, LP minted = requested amount
    assert_eq!(token_balance(&env, &s.pool.vault_x), 1_000_000);
    assert_eq!(token_balance(&env, &s.pool.vault_y), 4_000_000);
    assert_eq!(token_balance(&env, &s.user_lp), 1_000_000);
}

#[test]
fn test_deposit_proportional() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    // 500k more LP against 1M supply
    let supply = 1_000_000u64;
    let want_x = expected_deposit_ceil(500_000, 1_000_000, supply);
    let want_y = expected_deposit_ceil(500_000, 4_000_000, supply);
    assert_eq!((want_x, want_y), (500_000, 2_000_000));

    let x_before = token_balance(&env, &s.user_x);
    let y_before = token_balance(&env, &s.user_y);

    let ix = deposit_ix(&env, &s.pool, &s.user.pubkey(), 500_000, want_x, want_y);
    send(&mut env.svm, &[ix], &s.user, &[]).unwrap();

    assert_eq!(token_balance(&env, &s.pool.vault_x), 1_500_000);
    assert_eq!(token_balance(&env, &s.pool.vault_y), 6_000_000);
    assert_eq!(token_balance(&env, &s.user_lp), 1_500_000);
    assert_eq!(token_balance(&env, &s.user_x), x_before - want_x);
    assert_eq!(token_balance(&env, &s.user_y), y_before - want_y);
}

#[test]
fn test_swap_x_for_y() {
    let mut env = setup();
    let fee = 100; // 1%
    let s = liquid_pool(&mut env, fee, 1_000_000, 4_000_000, 1_000_000);

    let amount_in = 100_000u64;
    let want_out = expected_out(amount_in, 1_000_000, 4_000_000, fee);
    assert!(want_out > 0);

    let x_before = token_balance(&env, &s.user_x);
    let y_before = token_balance(&env, &s.user_y);

    let ix = swap_ix(&env, &s.pool, &s.user.pubkey(), true, amount_in, want_out);
    send(&mut env.svm, &[ix], &s.user, &[]).unwrap();

    assert_eq!(token_balance(&env, &s.user_x), x_before - amount_in);
    assert_eq!(token_balance(&env, &s.user_y), y_before + want_out);
    assert_eq!(token_balance(&env, &s.pool.vault_x), 1_000_000 + amount_in);
    assert_eq!(token_balance(&env, &s.pool.vault_y), 4_000_000 - want_out);

    // k must not shrink (fees grow it)
    let k_before = 1_000_000u128 * 4_000_000u128;
    let k_after = token_balance(&env, &s.pool.vault_x) as u128
        * token_balance(&env, &s.pool.vault_y) as u128;
    assert!(k_after >= k_before, "k must not decrease");
}

#[test]
fn test_swap_y_for_x() {
    let mut env = setup();
    let fee = 30;
    let s = liquid_pool(&mut env, fee, 2_000_000, 2_000_000, 2_000_000);

    let amount_in = 250_000u64;
    let want_out = expected_out(amount_in, 2_000_000, 2_000_000, fee);

    let x_before = token_balance(&env, &s.user_x);
    let y_before = token_balance(&env, &s.user_y);

    let ix = swap_ix(&env, &s.pool, &s.user.pubkey(), false, amount_in, want_out);
    send(&mut env.svm, &[ix], &s.user, &[]).unwrap();

    assert_eq!(token_balance(&env, &s.user_y), y_before - amount_in);
    assert_eq!(token_balance(&env, &s.user_x), x_before + want_out);
}

#[test]
fn test_withdraw() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    let x_before = token_balance(&env, &s.user_x);
    let y_before = token_balance(&env, &s.user_y);

    // burn 40%
    let burn = 400_000u64;
    let supply = 1_000_000u64;
    let want_x = expected_withdraw_floor(burn, 1_000_000, supply);
    let want_y = expected_withdraw_floor(burn, 4_000_000, supply);
    assert_eq!((want_x, want_y), (400_000, 1_600_000));

    let ix = withdraw_ix(&env, &s.pool, &s.user.pubkey(), burn, want_x, want_y);
    send(&mut env.svm, &[ix], &s.user, &[]).unwrap();

    assert_eq!(token_balance(&env, &s.user_lp), supply - burn);
    assert_eq!(token_balance(&env, &s.pool.vault_x), 1_000_000 - want_x);
    assert_eq!(token_balance(&env, &s.pool.vault_y), 4_000_000 - want_y);
    assert_eq!(token_balance(&env, &s.user_x), x_before + want_x);
    assert_eq!(token_balance(&env, &s.user_y), y_before + want_y);
}

// negative tests

#[test]
fn test_initialize_rejects_high_fee() {
    let mut env = setup();
    let mint_x = create_mint(&mut env, 6);
    let mint_y = create_mint(&mut env, 6);
    let pool = pool_for(&env.program_id, 7, mint_x, mint_y);

    let ix = initialize_ix(&env, &pool, 10_001, None); // > 100%
    let payer = env.payer.insecure_clone();
    assert!(send(&mut env.svm, &[ix], &payer, &[]).is_err());
}

#[test]
fn test_initialize_rejects_identical_mints() {
    let mut env = setup();
    let mint_x = create_mint(&mut env, 6);
    let pool = pool_for(&env.program_id, 8, mint_x, mint_x);

    let ix = initialize_ix(&env, &pool, 30, None);
    let payer = env.payer.insecure_clone();
    assert!(send(&mut env.svm, &[ix], &payer, &[]).is_err());
}

#[test]
fn test_deposit_rejects_zero_amount() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    let ix = deposit_ix(&env, &s.pool, &s.user.pubkey(), 0, 1_000_000, 1_000_000);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}

#[test]
fn test_deposit_rejects_slippage() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    // cap y below the required 2M
    let ix = deposit_ix(&env, &s.pool, &s.user.pubkey(), 500_000, 500_000, 1_000_000);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}

#[test]
fn test_swap_rejects_zero_amount() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 30, 1_000_000, 4_000_000, 1_000_000);

    let ix = swap_ix(&env, &s.pool, &s.user.pubkey(), true, 0, 0);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}

#[test]
fn test_swap_rejects_slippage() {
    let mut env = setup();
    let fee = 30;
    let s = liquid_pool(&mut env, fee, 1_000_000, 4_000_000, 1_000_000);

    let amount_in = 100_000u64;
    let real_out = expected_out(amount_in, 1_000_000, 4_000_000, fee);
    // ask for more than the curve gives
    let ix = swap_ix(&env, &s.pool, &s.user.pubkey(), true, amount_in, real_out + 1);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}

#[test]
fn test_withdraw_rejects_slippage() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    // demand more x than the proportional share
    let want_x = expected_withdraw_floor(100_000, 1_000_000, 1_000_000);
    let ix = withdraw_ix(&env, &s.pool, &s.user.pubkey(), 100_000, want_x + 1, 0);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}

#[test]
fn test_withdraw_rejects_overdraw() {
    let mut env = setup();
    let s = liquid_pool(&mut env, 0, 1_000_000, 4_000_000, 1_000_000);

    // burn more than owned
    let ix = withdraw_ix(&env, &s.pool, &s.user.pubkey(), 2_000_000, 0, 0);
    assert!(send(&mut env.svm, &[ix], &s.user, &[]).is_err());
}
