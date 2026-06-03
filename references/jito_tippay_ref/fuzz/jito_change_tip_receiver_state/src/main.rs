// invariant_change_tip_receiver_updates_config
//
// cf-invariants-jito-tippayment fixture — change_tip_receiver_state_update class.
// Target: Crucible v0.2.0 (asymmetric-research/crucible).
//
// Structural invariant under test:
//
//     After every successful call to `change_tip_receiver`, the on-chain
//     `Config.tip_receiver` MUST equal the `new_tip_receiver` pubkey the
//     caller passed in. I.e. the rotation must be committed to state, not
//     just appear to succeed.
//
// Why this invariant, not lamport conservation:
//
//     The Solana SVM runtime enforces total-lamport conservation across
//     every instruction natively (any program that fails to balance
//     debits and credits has its tx rejected by the runtime). So an
//     invariant fixture cannot meaningfully observe
//     lamport-non-conservation — the runtime rejects the offending tx
//     before the fixture's `read_account` ever sees a discrepancy. A
//     user-meaningful structural invariant has to live in the
//     post-instruction state shape that the runtime does NOT police, e.g.
//     whether on-chain config tracks the caller-requested rotation.
//
// Setup (pre-bake — sidesteps the `initialize` flow):
//   1. Derive Config PDA + 8 TipPaymentAccount PDAs from the program ID
//      and the hardcoded seeds. Capture each bump.
//   2. Pre-fund recv_a + recv_b (rotation targets) and fee_payer (signs
//      tx; the Anchor `signer` field has no identity constraint, so
//      this works) as system-owned accounts.
//   3. Pre-bake the Config account directly with
//      Config { tip_receiver: recv_a, block_builder: recv_a,
//      block_builder_commission_pct: 0, bumps }. Bumps must match the
//      derived PDA bumps so ChangeTipReceiver's Anchor constraints
//      (bump = config.bumps.tip_payment_account_N) check out on every
//      call.
//   4. Pre-bake each TipPaymentAccount at rent_min + TIP_PER_ACCOUNT.
//      (Tips drain on each call; the SVM tx-runtime balance check
//      requires the drain to be matched by a credit. The drain is a
//      necessary side-effect, not the invariant under test.)
//
// Action surface:
//   - action_change_tip_receiver_swap — rotate Config.tip_receiver
//     between recv_a and recv_b. The fixture tracks
//     `expected_tip_receiver` and updates it on every reported
//     success. After each action the invariant reads on-chain
//     Config.tip_receiver and asserts it equals expected.
//
// Invariant assertion:
//   on-chain Config.tip_receiver == fixture.expected_tip_receiver.
//   Clean reference: holds (program writes Config.tip_receiver = new
//   on every successful call). Planted twin (program drops that write):
//   first successful call → on-chain Config.tip_receiver stays at the
//   initial recv_a while expected becomes recv_b → VIOLATION.

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
/// rent_min. drain_account moves these out on every change_tip_receiver
/// call; they fund the receiver-credit side of handle_payments so the
/// program's SVM balance check passes.
const TIP_PER_ACCOUNT: u64 = 1_000_000;

#[derive(Clone)]
struct JitoChangeTipReceiverStateFixture {
    ctx: TestContext,
    program_id: Pubkey,
    /// Initial Config.tip_receiver and Config.block_builder.
    recv_a: Rc<Keypair>,
    /// Rotation target.
    recv_b: Rc<Keypair>,
    /// Pays per-tx Solana fees and is the Anchor `signer` account in
    /// ChangeTipReceiver. The Anchor `signer` field has no identity
    /// constraint — any signer works.
    fee_payer: Rc<Keypair>,
    config_pda: Pubkey,
    tip_pdas: [Pubkey; 8],
    /// Fixture-side oracle: what the caller most recently asked
    /// Config.tip_receiver to become. The invariant asserts that on-chain
    /// Config.tip_receiver matches this. Initialized to recv_a at setup
    /// (matching the pre-baked Config); advances on every successful
    /// change_tip_receiver call.
    expected_tip_receiver: Pubkey,
    /// Tracks whether the next call's new_tip_receiver will be recv_b
    /// (true) or recv_a (false). Drives the ping-pong rotation.
    next_target_is_b: bool,
}

#[fuzz_fixture]
impl JitoChangeTipReceiverStateFixture {
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
            expected_tip_receiver: recv_a.pubkey(),
            next_target_is_b: true,
        }
    }

    /// Rotate Config.tip_receiver. The current on-chain tip_receiver
    /// is taken from `expected_tip_receiver` (which matches on-chain
    /// state on the clean reference), so `old_tip_receiver.key() ==
    /// config.tip_receiver` is satisfied on the clean reference for
    /// every call.
    pub fn action_change_tip_receiver_swap(&mut self) -> bool {
        let old_recv = self.expected_tip_receiver;
        let new_recv = if self.next_target_is_b {
            self.recv_b.pubkey()
        } else {
            self.recv_a.pubkey()
        };
        // Skip no-op rotations.
        if old_recv == new_recv {
            return false;
        }
        // block_builder constraint: must equal config.block_builder,
        // which was set to recv_a at pre-bake and never moves in this
        // fixture (no change_block_builder action).
        let block_builder = self.recv_a.pubkey();

        let result = self
            .ctx
            .program(self.program_id)
            .call(instruction::ChangeTipReceiver {})
            .accounts(accounts::ChangeTipReceiver {
                config: self.config_pda,
                old_tip_receiver: old_recv,
                new_tip_receiver: new_recv,
                block_builder,
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
            // From the caller's perspective the rotation committed.
            // Update the fixture oracle and flip the ping-pong.
            self.expected_tip_receiver = new_recv;
            self.next_target_is_b = !self.next_target_is_b;
        }
        result
    }
}

// change_tip_receiver_updates_config invariant.
//
// On-chain Config.tip_receiver must equal the fixture's
// expected_tip_receiver after every action. Clean: holds. Planted (the
// commit line is missing in the program): on-chain stays at initial
// recv_a while expected advances → VIOLATION.
#[invariant_test]
fn invariant_change_tip_receiver_updates_config(
    fixture: &mut JitoChangeTipReceiverStateFixture,
) {
    // 8-byte Anchor discriminator (Config has the standard `#[account]`
    // derive → 8-byte sha256("account:Config")[..8] prefix).
    let config_state = fixture
        .ctx
        .read_account_with_discriminator::<Config>(&fixture.config_pda, 8)
        .expect("config account exists (pre-baked) and deserializes");

    fuzz_assert_eq!(
        config_state.tip_receiver,
        fixture.expected_tip_receiver,
        "Config.tip_receiver drift: on-chain={} expected={} (caller's most-recent new_tip_receiver)",
        config_state.tip_receiver,
        fixture.expected_tip_receiver
    );
}
