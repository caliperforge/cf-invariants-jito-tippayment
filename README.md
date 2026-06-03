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

## What it tests — one invariant class

| Class | Invariant under test | Planted-bug site |
|---|---|---|
| `change_tip_receiver_state_update` | `invariant_change_tip_receiver_updates_config` — after every successful `change_tip_receiver` call, on-chain `Config.tip_receiver` equals the `new_tip_receiver` pubkey the caller passed in. | `programs/tip-payment/src/lib.rs::change_tip_receiver` — the `ctx.accounts.config.tip_receiver = ctx.accounts.new_tip_receiver.key();` commit line is dropped. The instruction's lamport-side effects (drain + credit) still run normally, so Solana's runtime balance check is not tripped; only the rotation never commits. |

**Why this invariant, not lamport conservation.** A natural first pick
for tip-payment is "total lamports across `{recv_a, recv_b, config, 8
tip_pdas}` are conserved by every successful call." We tried it
first; it does not work as a Crucible invariant because the Solana
SVM runtime enforces total-lamport conservation across every
instruction natively. Any program that fails to balance debits and
credits has its tx rejected by the runtime *before* the fixture's
`read_account` sees a discrepancy — so the invariant cannot
meaningfully observe a violation. A user-meaningful structural
invariant for this program has to live in the post-instruction state
shape that the runtime does NOT police. `Config.tip_receiver` —
whether the on-chain rotation actually commits — is exactly that
shape.

CI result on the published commit: `clean = 0` violations and
`planted >= 1` violation. The CI badge is the source of truth — if
it is red, the harness is broken.

## Repository layout

```
.
├── programs/tip-payment/                          # cf-invariants-jito-tippayment port (anchor-lang 1.0.1)
├── references/
│   ├── jito_tippay_ref/                           # clean baseline + Crucible fuzz fixture
│   │   ├── programs/tip-payment/                  # ported program (== port above)
│   │   └── fuzz/jito_change_tip_receiver_state/   # fuzz fixture
│   └── jito_tippay_ref_planted_change_tip_receiver_state/
│       ├── programs/tip-payment/                  # planted variant (1-line bug)
│       └── fuzz/jito_change_tip_receiver_state/   # synced fixture (same code as clean)
├── .github/workflows/ci.yml                       # CI: workspace check + build-sbf + harness matrix
├── Cargo.toml                                     # workspace
├── LICENSE                                        # Apache-2.0 (CaliperForge)
├── NOTICE                                         # Jito attribution + modification log
└── README.md
```

The fuzz-fixture source for the invariant lives once under
`references/jito_tippay_ref/fuzz/jito_change_tip_receiver_state/src/main.rs`;
CI copies the same source into the planted variant before the run, so
the only difference between a clean run and the planted run is the
`.so` binary loaded into LiteSVM.

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

# 4. Build the clean reference + planted twin.
for variant in jito_tippay_ref \
               jito_tippay_ref_planted_change_tip_receiver_state; do
    cargo build-sbf --tools-version v1.52 \
        --manifest-path "references/${variant}/programs/tip-payment/Cargo.toml"
done

# 5. Build + install Crucible CLI from source.
(cd ../crucible && cargo install --path crates/crucible-fuzz-cli --locked)

# 6. Run the harness on the clean pair (expect no FUZZ_FINDING line).
(cd references/jito_tippay_ref/fuzz/jito_change_tip_receiver_state && \
    crucible run jito_tip_payment invariant_change_tip_receiver_updates_config \
        --release --timeout 30)

# 7. Same invariant against the planted twin (expect a FUZZ_FINDING within ~1s).
(cd references/jito_tippay_ref_planted_change_tip_receiver_state/fuzz/jito_change_tip_receiver_state && \
    crucible run jito_tip_payment invariant_change_tip_receiver_updates_config \
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
