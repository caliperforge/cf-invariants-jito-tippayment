# cf-invariants-jito-tippayment

**An invariant-fuzzing harness for the [Jito tip-payment program](https://github.com/jito-foundation/jito-programs/tree/master/mev-programs/programs/tip-payment), run on [Crucible](https://github.com/asymmetric-research/crucible).**

cf-invariants-jito-tippayment is a focused harness, not a new fuzzer. It
ports the upstream Jito tip-payment program from `anchor-lang` 0.31.1
to `anchor-lang` 1.0.1 so it can be driven by Crucible v0.2.0 (LibAFL
+ LiteSVM), then runs an invariant class against a clean reference
and a single-site planted-bug twin. Every push, CI rebuilds both
program variants and asserts `clean = 0` violations and `planted >= 1`
violation.

This is a sibling artifact to
[cf-invariants-jito](https://github.com/caliperforge/cf-invariants-jito)
(Jito tip-distribution), shipped by the same operator. It is the
*second* real Jito program harnessed under the same anchor-lang 1.0.1
/ Crucible v0.2.0 / platform-tools v1.52 rails — proof that the rails
generalize beyond a single target.

---

## Scope — what Jito tip-payment is, what this harness covers

The Jito tip-payment program is the on-chain piece of the
[Jito](https://www.jito.network/) MEV-redistribution stack on Solana
that receives validator tips. Searchers tip into one of 8 hardcoded
`TipPaymentAccount` PDAs (parallelism); the validator then calls
`change_tip_receiver` or `change_block_builder` to drain those PDAs
and credit the current tip receiver + block builder.

The on-chain `Config` PDA stores the rotation state — the current
`tip_receiver`, the current `block_builder`, and the
`block_builder_commission_pct`. The two rotation instructions both
read this config, drain the 8 tip PDAs via `handle_payments`, then
write the new identity back into Config.

The upstream code lives at
`jito-foundation/jito-programs/mev-programs/programs/tip-payment`
and is licensed Apache-2.0.

This harness does not modify the production program. It targets the
**invariant surface** of `change_tip_receiver` — the structural
property that must hold no matter what call sequence is fuzzed — and
proves the harness can confirm the property on the clean reference
and catch a deliberately planted regression.

## What it tests — three invariant classes

| Class | Invariant under test | Planted-bug site |
|---|---|---|
| `change_tip_receiver_state_update` | `invariant_change_tip_receiver_updates_config` — after every successful `change_tip_receiver` call, on-chain `Config.tip_receiver` equals the `new_tip_receiver` pubkey the caller passed in. | `programs/tip-payment/src/lib.rs::change_tip_receiver` — the `ctx.accounts.config.tip_receiver = ctx.accounts.new_tip_receiver.key();` commit line is dropped. The instruction's lamport-side effects (drain + credit) still run normally, so Solana's runtime balance check is not tripped; only the rotation never commits. |
| `change_block_builder_state_update` | `invariant_change_block_builder_updates_config` — after every successful `change_block_builder` call, on-chain `Config.block_builder` equals the `new_block_builder` argument AND `Config.block_builder_commission_pct` equals the `block_builder_commission` argument. | `programs/tip-payment/src/lib.rs::change_block_builder` — the `ctx.accounts.config.block_builder = ctx.accounts.new_block_builder.key();` commit line is dropped. The commission write below it still runs, so the lamport balance check passes; only the block-builder rotation never commits. |
| `block_builder_commission_pct_bounds` | `invariant_block_builder_commission_pct_bound` — at all times, on-chain `Config.block_builder_commission_pct` is in `[0, 100]`. (The program enforces percentage via `require_gte!(100, …)`, not basis points.) | `programs/tip-payment/src/lib.rs::change_block_builder` — the `require_gte!(100, block_builder_commission, …)` gate is dropped. The instruction then accepts any caller-supplied `u64` and commits it to `Config.block_builder_commission_pct`. |

**Why this invariant set, not lamport conservation.** A natural first pick
for tip-payment is "total lamports across `{recv_a, recv_b, config, 8
tip_pdas}` are conserved by every successful call." We tried it
first; it does not work as a Crucible invariant because the Solana
SVM runtime enforces total-lamport conservation across every
instruction natively. Any program that fails to balance debits and
credits has its tx rejected by the runtime *before* the fixture's
`read_account` sees a discrepancy — so the invariant cannot
meaningfully observe a violation. A user-meaningful structural
invariant for this program has to live in the post-instruction state
shape that the runtime does NOT police: `Config.tip_receiver` /
`Config.block_builder` (whether the on-chain rotation actually
commits) and `Config.block_builder_commission_pct` (whether the
percentage gate holds).

CI result on the published commit: `clean = 0` violations and
`planted >= 1` violation on each of the three class pairs. The CI
badge is the source of truth — if it is red, the harness is broken.

## Repository layout

```
.
├── programs/tip-payment/                          # cf-invariants-jito-tippayment port (anchor-lang 1.0.1)
├── references/
│   ├── jito_tippay_ref/                                      # clean baseline + Crucible fuzz fixtures
│   │   ├── programs/tip-payment/                             # ported program (== port above)
│   │   ├── fuzz/jito_change_tip_receiver_state/              # fixture: change_tip_receiver_state_update
│   │   ├── fuzz/jito_change_block_builder_state/             # fixture: change_block_builder_state_update
│   │   └── fuzz/jito_block_builder_commission_bounds/        # fixture: block_builder_commission_pct_bounds
│   ├── jito_tippay_ref_planted_change_tip_receiver_state/    # planted twin (drops the tip-receiver commit)
│   ├── jito_tippay_ref_planted_change_block_builder_state/   # planted twin (drops the block-builder commit)
│   └── jito_tippay_ref_planted_block_builder_commission/     # planted twin (drops the require_gte! gate)
├── .github/workflows/ci.yml                       # CI: workspace check + build-sbf + harness matrix
├── Cargo.toml                                     # workspace
├── LICENSE                                        # Apache-2.0 (CaliperForge)
├── NOTICE                                         # Jito attribution + modification log
└── README.md
```

Each invariant class has one fixture source-of-truth under
`references/jito_tippay_ref/fuzz/<class>/src/main.rs`; CI copies it
into the matching planted variant before the run, so the only
difference between a clean run and the planted run is the `.so`
binary loaded into LiteSVM.

## Pinned toolchain

These are the versions CI builds against on every push (see
[`.github/workflows/ci.yml`](./.github/workflows/ci.yml)). Pins are
inherited from the sister cf-invariants-jito project's CI-green stack:

- Rust **stable**.
- `anchor-lang` **1.0.1** — matches Crucible v0.2.0's workspace.
- Upstream [Crucible](https://github.com/asymmetric-research/crucible) **v0.2.0** — built from source in CI (`cargo install --path crates/crucible-fuzz-cli`).
- Anza / Solana CLI **v2.1.21** for `cargo-build-sbf`.
- Solana platform-tools **v1.52** (passed as `--tools-version v1.52`;
  Crucible v0.2.0 deps require `edition2024` support, which earlier
  platform-tools' rustc cannot build).
- `solana-sdk-ids` **3** (modular replacement for upstream's
  `solana-program = "2.2"` for the `loader_v4` / `bpf_loader` /
  `sysvar` / `config` / `secp256r1_program` / `native_loader` ID
  modules used in `is_program`/`is_sysvar`/`is_config`).

The fuzz `Cargo.toml` references Crucible via path dep at
`../../../../../crucible/...`, i.e. `<repo-root>/../crucible`. CI
clones Crucible v0.2.0 to that sibling path before the harness step.
For local reproduction, do the same.

## Reproduce from a fresh clone

CI runs exactly the steps below on every push. Local reproduction is
optional and requires the toolchain above installed and on `PATH`.

```sh
# 1. Clone this repo + Crucible v0.2.0 as a sibling.
git clone https://github.com/caliperforge/cf-invariants-jito-tippayment.git
git clone --depth 1 --branch v0.2.0 \
    https://github.com/asymmetric-research/crucible.git
cd cf-invariants-jito-tippayment

# 2. Workspace check (also runs in CI as the workspace-check job).
cargo check --workspace --locked || cargo check --workspace

# 3. Build the cf-invariants-jito-tippayment port (SBPF).
cargo build-sbf --tools-version v1.52 \
    --manifest-path programs/tip-payment/Cargo.toml

# 4. Build the clean reference + the three planted twins.
for variant in jito_tippay_ref \
               jito_tippay_ref_planted_change_tip_receiver_state \
               jito_tippay_ref_planted_change_block_builder_state \
               jito_tippay_ref_planted_block_builder_commission; do
    cargo build-sbf --tools-version v1.52 \
        --manifest-path "references/${variant}/programs/tip-payment/Cargo.toml"
done

# 5. Build + install Crucible CLI from source.
(cd ../crucible && cargo install --path crates/crucible-fuzz-cli --locked)

# 6. Run the three clean pairs (expect no FUZZ_FINDING / [VIOLATION] line).
(cd references/jito_tippay_ref/fuzz/jito_change_tip_receiver_state && \
    crucible run jito_tip_payment invariant_change_tip_receiver_updates_config \
        --release --timeout 30)
(cd references/jito_tippay_ref/fuzz/jito_change_block_builder_state && \
    crucible run jito_tip_payment invariant_change_block_builder_updates_config \
        --release --timeout 30)
(cd references/jito_tippay_ref/fuzz/jito_block_builder_commission_bounds && \
    crucible run jito_tip_payment invariant_block_builder_commission_pct_bound \
        --release --timeout 30)

# 7. Same invariants against the planted twins (expect violations within ~1s each).
(cd references/jito_tippay_ref_planted_change_tip_receiver_state/fuzz/jito_change_tip_receiver_state && \
    crucible run jito_tip_payment invariant_change_tip_receiver_updates_config \
        --release --timeout 30)
(cd references/jito_tippay_ref_planted_change_block_builder_state/fuzz/jito_change_block_builder_state && \
    crucible run jito_tip_payment invariant_change_block_builder_updates_config \
        --release --timeout 30)
(cd references/jito_tippay_ref_planted_block_builder_commission/fuzz/jito_block_builder_commission_bounds && \
    crucible run jito_tip_payment invariant_block_builder_commission_pct_bound \
        --release --timeout 30)
```

CI runs steps 2 through 7 on every push. The scorecard captures (raw
Crucible output, ANSI-stripped) are uploaded as the
`crucible-scorecards` workflow artifact and written under
`findings/<invariant>_<variant>/scorecard.md` inside the runner.
`findings/` is gitignored; the CI artifact is the canonical record.
See [`.github/workflows/ci.yml`](./.github/workflows/ci.yml) for the
canonical sequence.

## What this is not

- **Not a fork of Crucible.** Crucible is the harness;
  cf-invariants-jito-tippayment is a target + fuzz fixture that runs
  on top of it. Credit for the LiteSVM execution rails and the
  IDL-driven fuzzing plumbing belongs to Asymmetric Research.
- **Not a Jito security audit.** The planted twin is a synthetic
  single-site regression authored to prove the corresponding
  invariant class fires. No claim is made about the production Jito
  program's security from this harness alone.
- **Not a formal-verification tool.** Randomized invariant fuzzing,
  not proofs.

## Credits

- Upstream tip-payment program: [Jito Foundation](https://www.jito.network/) — `jito-foundation/jito-programs` (Apache-2.0).
- Fuzz harness: [Crucible](https://github.com/asymmetric-research/crucible) by [Asymmetric Research](https://www.asymmetric.re/) (MIT, v0.2.0).
- Anchor framework: [coral-xyz/anchor](https://github.com/coral-xyz/anchor) (Apache-2.0).

## Reporting issues, security contact

Open an issue on this GitHub repository, or contact
[michael@caliperforge.com](mailto:michael@caliperforge.com).

## License

Apache-2.0. See [`LICENSE`](./LICENSE) and [`NOTICE`](./NOTICE). The
`NOTICE` file preserves Jito's upstream Apache-2.0 attribution and
describes the modifications relative to upstream.

---

cf-invariants-jito-tippayment is operated by Michael Moffett under the
CaliperForge banner. CaliperForge is a sole-operator engineering studio.

This scaffold was built with AI assistance. Authored and reviewed by
Michael Moffett, operator at CaliperForge. Full policy at
[caliperforge.com/ai-disclosure](https://caliperforge.com/ai-disclosure).
