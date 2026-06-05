// invariant_block_builder_commission_pct_bound
//
// cf-invariants-jito-tippayment fixture — block_builder_commission_pct_bounds
// class. Target: Crucible v0.2.0.
//
// Structural invariant under test:
//
//     At all times after `initialize`, on-chain
//     Config.block_builder_commission_pct <= 100. The program's
//     `change_block_builder` instruction is the only writer of this field
//     and gates its argument on `require_gte!(100, …)` before committing;
//     `initialize` seeds the field to 0; `change_tip_receiver` does not
//     touch it. So the bound is a true program post-condition.
//
//     Note: the program enforces percentage (<= 100), NOT basis points
//     (<= 10000). The invariant + class name reflect that.
//
// Setup (pre-bake — same scaffold as the two existing classes):
//   1. Derive Config PDA + 8 TipPaymentAccount PDAs from the program ID
//      and the hardcoded seeds. Capture each bump.
//   2. Pre-fund recv_a / recv_b (rotation targets — used as
//      block_builder identities here) and fee_payer as system-owned
//      accounts.
//   3. Pre-bake the Config account with Config { tip_receiver: recv_a,
//      block_builder: recv_a, block_builder_commission_pct: 0, bumps }.
//   4. Pre-bake each TipPaymentAccount at rent_min + TIP_PER_ACCOUNT.
//
// Action surface:
//   - action_change_block_builder_commission — calls
//     `change_block_builder` with a fuzzed commission value in
//     [0, 255]. The clean program rejects values > 100 at instruction
//     entry (require_gte!), so those calls fail and the fixture oracle
//     stays untouched — Config.block_builder_commission_pct never
//     exceeds 100. The planted twin (the require_gte! check is
//     dropped) accepts the same out-of-range value and commits it
//     into Config, surfacing the violation.
//
// Invariant assertion:
//   on-chain Config.block_builder_commission_pct <= 100. Clean:
//   holds (require_gte! gates the write). Planted (require_gte!
//   dropped): first successful call with commission > 100 → on-chain
//   value > 100 → VIOLATION.

#![allow(unused_imports)]

use crucible_fuzzer::anchor_lang::system_program;
use crucible_fuzzer::*;
use ::jito_tip_payment::*;
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use std::rc::Rc;

const INITIAL_BALANCE: u64 = 10_000_000_000;
const TIP_PER_ACCOUNT: u64 = 1_000_000;

#[derive(Clone)]
struct JitoBlockBuilderCommissionBoundsFixture {
    ctx: TestContext,
    program_id: Pubkey,
    /// Initial Config.tip_receiver and Config.block_builder. Tip_receiver
    /// stays static; block_builder rotates so each call satisfies
    /// `old_block_builder.key() == config.block_builder` on the clean
    /// reference.
    recv_a: Rc<Keypair>,
    recv_b: Rc<Keypair>,
    fee_payer: Rc<Keypair>,
    config_pda: Pubkey,
    tip_pdas: [Pubkey; 8],
    /// Tracks the on-chain Config.block_builder for the clean
    /// reference, so each call passes the correct `old_block_builder`
    /// argument. Initialized to recv_a; flipped on every successful
    /// rotation.
    current_block_builder: Pubkey,
    next_target_is_b: bool,
}

#[fuzz_fixture]
impl JitoBlockBuilderCommissionBoundsFixture {
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
            current_block_builder: recv_a.pubkey(),
            next_target_is_b: true,
        }
    }

    /// Call `change_block_builder` with a fuzzed commission in
    /// [0, 255]. Values > 100 are rejected at instruction entry on
    /// the clean reference (require_gte! check); the planted twin
    /// (require_gte! dropped) accepts and commits the out-of-range
    /// value into Config, tripping the invariant.
    pub fn action_change_block_builder_commission(
        &mut self,
        #[range(0..=255)] commission: u64,
    ) -> bool {
        let old_bb = self.current_block_builder;
        let new_bb = if self.next_target_is_b {
            self.recv_b.pubkey()
        } else {
            self.recv_a.pubkey()
        };
        if old_bb == new_bb {
            return false;
        }
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
            self.current_block_builder = new_bb;
            self.next_target_is_b = !self.next_target_is_b;
        }
        result
    }
}

// block_builder_commission_pct_bound invariant.
//
// On-chain Config.block_builder_commission_pct must remain in [0, 100]
// at all times. Clean: holds (require_gte! gates the write). Planted
// (require_gte! dropped): first successful call with commission > 100
// → on-chain value > 100 → VIOLATION.
#[invariant_test]
fn invariant_block_builder_commission_pct_bound(
    fixture: &mut JitoBlockBuilderCommissionBoundsFixture,
) {
    let config_state = fixture
        .ctx
        .read_account_with_discriminator::<Config>(&fixture.config_pda, 8)
        .expect("config account exists (pre-baked) and deserializes");

    fuzz_assert_le!(
        config_state.block_builder_commission_pct,
        100u64,
        "Config.block_builder_commission_pct out-of-bound: on-chain={} (must be <= 100; require_gte! gate must hold)",
        config_state.block_builder_commission_pct
    );
}
