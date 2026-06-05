// invariant_change_block_builder_updates_config
//
// cf-invariants-jito-tippayment fixture — change_block_builder_state_update
// class. Sister to change_tip_receiver_state_update; mirrors its shape with
// the rotation roles swapped (this class moves Config.block_builder +
// Config.block_builder_commission_pct, holds Config.tip_receiver static).
// Target: Crucible v0.2.0.
//
// Structural invariant under test:
//
//     After every successful call to `change_block_builder`, BOTH on-chain
//     Config.block_builder == new_block_builder.key() (the argument) AND
//     Config.block_builder_commission_pct == block_builder_commission (the
//     argument). Both writes are unconditional on the program's success
//     path; the rotation must be committed to state, not just appear to
//     succeed.
//
// Setup (pre-bake — same scaffold as change_tip_receiver_state):
//   1. Derive Config PDA + 8 TipPaymentAccount PDAs from the program ID
//      and the hardcoded seeds. Capture each bump.
//   2. Pre-fund recv_a / recv_b (rotation targets — used as
//      block_builder identities here) and fee_payer as system-owned
//      accounts.
//   3. Pre-bake the Config account with Config { tip_receiver: recv_a,
//      block_builder: recv_a, block_builder_commission_pct: 0, bumps }.
//      recv_a fills both roles: tip_receiver is held static for this
//      class (satisfies the `tip_receiver.key() == config.tip_receiver`
//      constraint on every call) and is the initial block_builder
//      (rotation starts from recv_a).
//   4. Pre-bake each TipPaymentAccount at rent_min + TIP_PER_ACCOUNT
//      (handle_payments drains them on each call; the credit lands on
//      tip_receiver / old_block_builder so the SVM balance check
//      passes).
//
// Action surface:
//   - action_change_block_builder_swap — rotate Config.block_builder
//     between recv_a and recv_b, with a fuzzed commission_pct in
//     [0, 100]. The clean program rejects commission > 100 via
//     require_gte!, so the action stays in-range to keep the
//     state-update path under test (a separate class —
//     block_builder_commission_pct_bounds — covers the > 100 reject
//     surface). The fixture tracks expected_block_builder +
//     expected_commission_pct and updates them on every reported
//     success.
//
// Invariant assertion:
//   on-chain Config.block_builder == fixture.expected_block_builder AND
//   on-chain Config.block_builder_commission_pct ==
//   fixture.expected_commission_pct. Clean reference: holds (program
//   commits both writes on every successful call). Planted twin (the
//   `config.block_builder = new_block_builder.key()` write is dropped):
//   first successful rotation → on-chain stays at recv_a while expected
//   becomes recv_b → VIOLATION.

#![allow(unused_imports)]

use crucible_fuzzer::anchor_lang::system_program;
use crucible_fuzzer::*;
use ::jito_tip_payment::*;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use std::rc::Rc;

const INITIAL_BALANCE: u64 = 10_000_000_000;
/// Tip lamports added to each of the 8 TipPaymentAccount PDAs above
/// rent_min. drain_account moves these out on every change_block_builder
/// call; they fund the receiver-credit side of handle_payments so the
/// program's SVM balance check passes.
const TIP_PER_ACCOUNT: u64 = 1_000_000;

#[derive(Clone)]
struct JitoChangeBlockBuilderStateFixture {
    ctx: TestContext,
    program_id: Pubkey,
    /// Initial Config.tip_receiver and Config.block_builder. Tip_receiver
    /// is held static for this class — the rotation moves block_builder.
    recv_a: Rc<Keypair>,
    /// Rotation target.
    recv_b: Rc<Keypair>,
    /// Pays per-tx Solana fees and is the Anchor `signer` account in
    /// ChangeBlockBuilder. The Anchor `signer` field has no identity
    /// constraint — any signer works.
    fee_payer: Rc<Keypair>,
    config_pda: Pubkey,
    tip_pdas: [Pubkey; 8],
    /// Fixture-side oracle: what the caller most recently asked
    /// Config.block_builder to become. Initialized to recv_a (matching
    /// the pre-baked Config); advances on every successful
    /// change_block_builder call.
    expected_block_builder: Pubkey,
    /// Fixture-side oracle: what the caller most recently asked
    /// Config.block_builder_commission_pct to become. Initialized to 0
    /// (matching the pre-baked Config); advances on every successful
    /// change_block_builder call.
    expected_commission_pct: u64,
    /// Tracks whether the next call's new_block_builder will be recv_b
    /// (true) or recv_a (false). Drives the ping-pong rotation.
    next_target_is_b: bool,
}

#[fuzz_fixture]
impl JitoChangeBlockBuilderStateFixture {
    pub fn setup() -> Self {
        let mut ctx = TestContext::new();
        let program_id = Pubkey::new_from_array(ID.to_bytes());
        ctx.add_program(&program_id, "../../target/deploy/jito_tip_payment.so")
            .unwrap();

        let recv_a = Rc::new(Keypair::new());
        let recv_b = Rc::new(Keypair::new());
        let fee_payer = Rc::new(Keypair::new());

        ctx.create_account()
            .pubkey(recv_a.pubkey())
            .lamports(INITIAL_BALANCE)
            .owner(system_program::ID)
            .create()
            .unwrap();
        ctx.create_account()
            .pubkey(recv_b.pubkey())
            .lamports(INITIAL_BALANCE)
            .owner(system_program::ID)
            .create()
            .unwrap();
        ctx.create_account()
            .pubkey(fee_payer.pubkey())
            .lamports(INITIAL_BALANCE)
            .owner(system_program::ID)
            .create()
            .unwrap();

        let (config_pda, config_bump) =
            Pubkey::find_program_address(&[CONFIG_ACCOUNT_SEED], &program_id);
        let tip_seeds: [&[u8]; 8] = [
            TIP_ACCOUNT_SEED_0,
            TIP_ACCOUNT_SEED_1,
            TIP_ACCOUNT_SEED_2,
            TIP_ACCOUNT_SEED_3,
            TIP_ACCOUNT_SEED_4,
            TIP_ACCOUNT_SEED_5,
            TIP_ACCOUNT_SEED_6,
            TIP_ACCOUNT_SEED_7,
        ];
        let mut tip_pdas: [Pubkey; 8] = [Pubkey::default(); 8];
        let mut tip_bumps: [u8; 8] = [0; 8];
        for i in 0..8 {
            let (pda, bump) = Pubkey::find_program_address(&[tip_seeds[i]], &program_id);
            tip_pdas[i] = pda;
            tip_bumps[i] = bump;
        }

        let bumps = InitBumps {
            config: config_bump,
            tip_payment_account_0: tip_bumps[0],
            tip_payment_account_1: tip_bumps[1],
            tip_payment_account_2: tip_bumps[2],
            tip_payment_account_3: tip_bumps[3],
            tip_payment_account_4: tip_bumps[4],
            tip_payment_account_5: tip_bumps[5],
            tip_payment_account_6: tip_bumps[6],
            tip_payment_account_7: tip_bumps[7],
        };
        let config_state = Config {
            tip_receiver: recv_a.pubkey(),
            block_builder: recv_a.pubkey(),
            block_builder_commission_pct: 0,
            bumps,
        };
        use crucible_fuzzer::anchor_lang::prelude::Rent;
        let rent = Rent::default();
        let rent_min_for_config = rent.minimum_balance(Config::SIZE);
        ctx.create_account()
            .pubkey(config_pda)
            .lamports(rent_min_for_config)
            .owner(program_id)
            .size(Config::SIZE)
            .create()
            .unwrap();
        ctx.write_anchor_account(&config_pda, &config_state).unwrap();

        let rent_min_for_tip = rent.minimum_balance(TipPaymentAccount::SIZE);
        for i in 0..8 {
            ctx.create_account()
                .pubkey(tip_pdas[i])
                .lamports(rent_min_for_tip + TIP_PER_ACCOUNT)
                .owner(program_id)
                .size(TipPaymentAccount::SIZE)
                .create()
                .unwrap();
            ctx.write_anchor_account(&tip_pdas[i], &TipPaymentAccount {})
                .unwrap();
        }

        Self {
            ctx,
            program_id,
            recv_a: recv_a.clone(),
            recv_b,
            fee_payer,
            config_pda,
            tip_pdas,
            expected_block_builder: recv_a.pubkey(),
            expected_commission_pct: 0,
            next_target_is_b: true,
        }
    }

    /// Rotate Config.block_builder. The current on-chain block_builder
    /// is taken from `expected_block_builder` (which matches on-chain
    /// state on the clean reference), so `old_block_builder.key() ==
    /// config.block_builder` is satisfied on the clean reference for
    /// every call. commission is constrained to [0, 100] to stay
    /// inside the program's `require_gte!` bound.
    pub fn action_change_block_builder_swap(
        &mut self,
        #[range(0..=100)] commission: u64,
    ) -> bool {
        let old_bb = self.expected_block_builder;
        let new_bb = if self.next_target_is_b {
            self.recv_b.pubkey()
        } else {
            self.recv_a.pubkey()
        };
        if old_bb == new_bb {
            return false;
        }
        // tip_receiver constraint: must equal config.tip_receiver,
        // which was set to recv_a at pre-bake and is never moved by
        // this class.
        let tip_receiver = self.recv_a.pubkey();

        let result = self
            .ctx
            .program(self.program_id)
            .call(instruction::ChangeBlockBuilder {
                block_builder_commission: commission,
            })
            .accounts(accounts::ChangeBlockBuilder {
                config: self.config_pda,
                tip_receiver,
                old_block_builder: old_bb,
                new_block_builder: new_bb,
                tip_payment_account_0: self.tip_pdas[0],
                tip_payment_account_1: self.tip_pdas[1],
                tip_payment_account_2: self.tip_pdas[2],
                tip_payment_account_3: self.tip_pdas[3],
                tip_payment_account_4: self.tip_pdas[4],
                tip_payment_account_5: self.tip_pdas[5],
                tip_payment_account_6: self.tip_pdas[6],
                tip_payment_account_7: self.tip_pdas[7],
                signer: self.fee_payer.pubkey(),
            })
            .signers(&[&*self.fee_payer])
            .send()
            .map(|o| o.is_success())
            .unwrap_or(false);

        if result {
            self.expected_block_builder = new_bb;
            self.expected_commission_pct = commission;
            self.next_target_is_b = !self.next_target_is_b;
        }
        result
    }
}

// change_block_builder_updates_config invariant.
//
// On-chain Config.block_builder must equal the fixture's
// expected_block_builder AND Config.block_builder_commission_pct must
// equal expected_commission_pct after every action. Clean: holds.
// Planted (the block_builder commit line is missing in the program):
// on-chain stays at initial recv_a while expected advances → VIOLATION.
#[invariant_test]
fn invariant_change_block_builder_updates_config(
    fixture: &mut JitoChangeBlockBuilderStateFixture,
) {
    let config_state = fixture
        .ctx
        .read_account_with_discriminator::<Config>(&fixture.config_pda, 8)
        .expect("config account exists (pre-baked) and deserializes");

    fuzz_assert_eq!(
        config_state.block_builder,
        fixture.expected_block_builder,
        "Config.block_builder drift: on-chain={} expected={} (caller's most-recent new_block_builder)",
        config_state.block_builder,
        fixture.expected_block_builder
    );
    fuzz_assert_eq!(
        config_state.block_builder_commission_pct,
        fixture.expected_commission_pct,
        "Config.block_builder_commission_pct drift: on-chain={} expected={} (caller's most-recent block_builder_commission)",
        config_state.block_builder_commission_pct,
        fixture.expected_commission_pct
    );
}
