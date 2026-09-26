# ERA fork of rgb-lib

`ERAWLT/rgb-lib`, branch `era/configurable-derivation`, is
[UTEXO-Protocol/rgb-lib](https://github.com/UTEXO-Protocol/rgb-lib) at tag
`v0.3.0-beta.34` (`62a8c3a`, crate version `0.3.0-beta.7`) plus a short series of
commits. Its only consumer is the ERA Wallet companion app, which links it through
`packages/era_rgb` (flutter_rust_bridge) with `default-features = false` and the
features `esplora` + `vss`, pinned **by rev** of this branch.

Rules for the branch:

- **Never rewrite or force-push it.** The app pins a commit; a rev that is no longer
  reachable from a ref breaks every build of that app version. The branch has not been
  rewritten since `6ce375e` (2026-09-25); the one earlier rewrite, a non-fast-forward
  push over `ca5f6b7`, predates any pin.
- **Push only the branch to origin, never tags**: `git push origin era/configurable-derivation`,
  never `--tags`, `--follow-tags` or `--mirror`, and never a UTEXO tag. A UTEXO `v*`
  tag pushed here runs `release.yml` as it is in the tagged commit, which has no owner
  condition (see [§2](#2-ci-1d26404-extended-in-f808c7f-and-07e16e5)). This clone
  holds UTEXO's tags from `git fetch utexo --tags`, including `v0.3.0-beta.43-bfa`,
  which origin does not have.
- A new UTEXO base gets a **new branch** (see [Carrying the series](#carrying-the-series-onto-a-new-utexo-tag)),
  never a rebase of this one.
- Commits are authored by people, with no AI co-author trailers.

## The series

| Commit | Subject | Upstream? |
|---|---|---|
| `6ce375e` | Add a configurable keychain layout to singlesig keys | To propose to UTEXO (draft [below](#pr-proposal-for-utexo), not sent) |
| `1d26404` | Run the fork's own checks on era branches only | Fork-only |
| `f808c7f` | Carry one TLS stack and no migration CLI in the library | Fork-only |
| `d82e21a` | Document the ERA fork of rgb-lib (this file) | Fork-only |
| `07e16e5` | Guard HTTP client construction and probe real https in CI | Fork-only |

Later commits that touch only this file are part of the series too.

## 1. Configurable keychain layout (`6ce375e`)

### Why

rgb-lib keeps the two sides of a singlesig wallet under two BIP-86 accounts:

| Side | Default path | Account xpub the host must supply |
|---|---|---|
| colored (RGB allocations) | `m/86'/827166'/0'/0/*` (`827167'` off mainnet) | `account_xpub_colored` at `m/86'/827166'/0'` |
| vanilla (plain BTC) | `m/86'/0'/0'/<vanilla_keychain>/*` (`1'` off mainnet) | `account_xpub_vanilla` at `m/86'/0'/0'` |

A watch-only host needs both account xpubs. A hardware wallet exports the standard
accounts it knows (BIP-44/49/84/86 at coin type `0'`) and nothing at `827166'`; adding
a new account export means a firmware release for every device in the field. With a
signer like that, rgb-lib cannot build the colored descriptor at all.

### What changes

- `SinglesigKeys` (`src/wallet/singlesig.rs`) gets three optional fields next to
  `vanilla_keychain`: `colored_keychain: Option<u8>`, `colored_coin_type: Option<u32>`,
  `vanilla_coin_type: Option<u32>`. In JSON / C-FFI they are `coloredKeychain`,
  `coloredCoinType`, `vanillaCoinType` with `camel_case`, and accept a number or a
  numeric string like the existing fields. `SinglesigKeys::with_keychain_layout(...)`
  sets them on keys built by `from_keys` / `from_keys_no_mnemonic`.
- `KeychainLayout::resolve` (`src/utils.rs`) turns those options into concrete coin
  types and keychains for the wallet's network and validates them: a coin type must
  be a valid hardened index (`< 2^31`), and the colored and vanilla sides must not
  resolve to the same (coin type, keychain) pair, because one BDK keychain feeding
  both sides would make colored UTXOs spendable as vanilla ones.
- `Error::InvalidKeychainLayout { details }` reports a rejected layout (also added to
  the uniffi UDL).
- `get_descriptors` / `get_descriptors_from_xpubs` build both descriptors from the
  resolved layout instead of the constants; the key origin in the descriptor (and so
  in every PSBT) is the layout's path.
- `wallet_manifest.json` stores `colored_keychain` / `colored_coin_type` /
  `vanilla_coin_type` **only when they differ from the default** (serde
  `skip_serializing_if`), so manifests of default-layout wallets stay byte-identical;
  a layout that changes between `new` and `load` is a `WalletSettingMismatch`.
- New public `rgb_lib::utils::get_account_data_at_coin_type(network, mnemonic,
  coin_type, witness_version)`; `get_account_data` delegates to it. Mnemonic-based
  hosts use it to prepare keys for a custom layout.
- Multisig is untouched: cosigners build one shared descriptor from the constants and
  would first have to agree on a layout.

### What does not change

- With all three fields unset, descriptors, derivation paths and manifests are exactly
  what they were (`tests_keychain_layout::default_layout_descriptors_unchanged`).
- The layout is local to the wallet. Invoices, consignments, ACKs and witness
  transactions carry no derivation paths, so a custom-layout wallet transfers with
  default-layout wallets in both directions. Checked live on UTEXO's signet on
  2026-09-25: a watch-only wallet in the ERA layout received from and sent to a
  default-layout wallet, both transfers `Settled` on both sides.

### ERA's layout

One exported account, `m/86'/0'/0'` (as `tpub` off mainnet), passed as both account
xpubs, with `colored_coin_type = 0`, `vanilla_coin_type = 0` (the device exports no
`1'` account, so 0 on every network), `colored_keychain = 9`, `vanilla_keychain = 10`:

```
colored  tr([fp/86'/0'/0'/9]xpub…/*)
vanilla  tr([fp/86'/0'/0'/10]xpub…/*)
```

Keychains 9 and 10 stay clear of the receive/change chains (0/1) a regular Bitcoin
wallet on the same account uses.

### Tests

`cargo test --locked --lib --features esplora,vss -- tests_keychain_layout` (5 tests:
defaults per network, shared-keychain and non-hardenable rejections, the single-account
layout from xpubs and from a mnemonic, unchanged default descriptors).

## 2. CI (`1d26404`, extended in `f808c7f` and `07e16e5`)

`.github/workflows/era.yml` has two jobs.

- **check** runs on push / PR to `era/**` and on manual dispatch: rustfmt, `cargo check`
  with the app's feature set and with the upstream default on top, a dependency guard
  over the app's mobile targets, the HTTP client guard (below), the unit tests above
  plus the REST-client TLS test and offline wallet tests, and the migration crate.
  Every cargo call is `--locked`: a fresh resolution fails on the yanked
  `secp256k1 0.32.0-beta.2` that `rgb-consensus 0.11.1-rc.11` requires.
- **https** runs on manual dispatch and on a weekly schedule, with `continue-on-error`:
  the two tests that make real TLS handshakes, to UTEXO's signet RGB proxy
  (`rest_client_talks_to_a_public_https_proxy`, through `rest_client_builder`'s
  config) and to its signet Esplora indexer (`wallet::test::new::signet_esplora_success`,
  through minreq). They depend on third-party hosts, so they stay out of **check**; a
  red run is a reason to look, not a broken branch. The step insists on exactly two
  passes, because `--exact` with a renamed test runs nothing and passes. GitHub fires
  `schedule` only from the default branch, so while that is UTEXO's `dev` the weekly
  run never happens and the job runs only when dispatched.

**The HTTP client guard.** reqwest is compiled with `rustls-no-provider`, so a client
built anywhere but `api::rest_client_builder` compiles and fails at runtime: reqwest's
own TLS path panics with "No provider set" when no process-wide `CryptoProvider` is
installed, and otherwise verifies through `rustls-platform-verifier`, which on Android
fails the handshake unless the app initialised it over JNI. The step fails when
`RestClient::new/builder/default`, `reqwest::(blocking::)Client::` /
`ClientBuilder::` or `reqwest::(blocking::)get(` appears in `src/` outside
`src/api/mod.rs`, and when the pattern stops matching the one construction there. It
is a grep, not a type check: an alias (`use reqwest::blocking::Client as Http;
Http::new()`) escapes it.

### The inherited workflows

They are restricted, not removed, so merges from upstream stay simple. The
restriction is an owner condition (`github.repository_owner == 'UTEXO-Protocol'`)
added to their jobs **in the files on `era/**` commits only**. GitHub reads a
workflow file from a different commit for each event, so the condition protects
PRs into `era/**`, dispatches on an `era/**` ref and `v*` tags on `era/**` commits,
and nothing else:

| Workflow | Trigger | Not covered by the owner condition |
|---|---|---|
| build, test, lint, format | push / PR to `utexo-master`, `dev`, `stage`, `master` | no condition needed: never fire on `era/**`, and we never push to those branches |
| claude-code-review, kimi-code-review | PR label / sync, PR comment mentioning the bot, dispatch | **every `issue_comment`**: GitHub runs it from the default branch, `dev`, whose file has no condition. kimi runs a third-party action pinned to `@main` with PR and issue write access |
| release | any `v*` tag, dispatch | **a `v*` tag on a UTEXO commit**: the file comes from the tagged commit. This repository mirrors UTEXO's tags; a run would publish a release on our repository (`contents: write`) and try to dispatch to UTEXO's binding repos with PATs we do not hold |

Also uncovered: a dispatch or a PR on a non-`era/**` ref. On 2026-09-26 nothing had
used any of this: the repository's Actions history held only `era.yml` runs, and the
Actions API listed `era.yml` as its only workflow. That is the current state, not a
control.

**The real control is the repository settings**, which apply to every ref and every
event. They need an admin of `ERAWLT/rgb-lib` (the lead):

1. Disable `release`, `claude-code-review` and `kimi-code-review`: Actions → the
   workflow → ⋯ → *Disable workflow*, or `gh workflow disable <file> -R ERAWLT/rgb-lib`.
   A disabled workflow does not run for any event on any ref. The Actions list shows
   a workflow only once GitHub has registered it; if the three are not listed, use 2.
2. Or, in effect, allow only `era.yml`. GitHub has no per-file allowlist, but it has
   one for actions: Settings → Actions → General → *Allow ERAWLT, and select
   non-ERAWLT, actions and reusable workflows*, with *Allow actions created by
   GitHub* unticked and exactly what `era.yml` uses listed: `actions/checkout@*`,
   `actions-rust-lang/setup-rust-toolchain@*`, `Swatinem/rust-cache@*` (called inside
   setup-rust-toolchain). Every job of the other three then stops at setup, because
   each uses an action outside the list (`actions/upload-artifact`,
   `softprops/action-gh-release`, `anthropics/claude-code-action`,
   `UTEXO-Protocol/kimi-actions`). A new action in `era.yml` then needs a line there.
3. Optionally, switch the default branch to `era/configurable-derivation`: an
   `issue_comment` then reads the file with the condition, and the weekly https run
   starts. It does nothing for a `v*` tag, which only 1 or 2 closes. If the series
   moves to a new `era/<name>` branch, the default branch has to follow.

On our side, the rule at the top: push the branch, never tags.

## 3. Dependency diet (`f808c7f`)

### Why

With `esplora` + `vss` on Android the library carried three crypto/TLS stacks
(vendored OpenSSL through `native-tls-vendored`, aws-lc through reqwest's default
rustls provider, ring through the Esplora and VSS clients) plus clap and
sqlx-postgres, which only the migration authoring CLI needs. TLS itself is still
required: until the app's loopback forwarder exists, rgb-lib reaches the RGB proxy,
the Esplora indexer and VSS over https directly.

### What changes

- **reqwest** is declared once, for every target, with `rustls-no-provider` instead of
  `native-tls-vendored` (Android) / `rustls` (elsewhere); its other defaults
  (`charset`, `http2`, `system-proxy`) are kept. The two target-specific tables are
  gone; they were also non-optional, which compiled reqwest into builds with no
  network feature.
- **`api::rest_client_builder`** builds every blocking HTTP client (RGB proxy, reject
  list, multisig hub, DFNS) with a preconfigured `rustls::ClientConfig`: ring provider,
  Mozilla roots from `webpki-roots`, ALPN `h2` + `http/1.1` as reqwest would offer.
- **Certificate verification uses the bundled Mozilla roots, not the platform
  verifier.** With `rustls-no-provider`, reqwest's own TLS path needs a process-wide
  `CryptoProvider` (without one, building a client panics with "No provider set") and,
  by default, the platform verifier: `rustls-platform-verifier`, which on Android has
  to be initialised over JNI with an application `Context` and needs its Kotlin
  component in the app's Gradle build; a missed init fails the first handshake. The
  prebuilt config removes both dependencies. The Esplora client (minreq) and the VSS
  client (bitreq) already verify against bundled webpki roots, so all three HTTP
  clients now trust the same store on every platform. Consequences: user-installed and
  enterprise CAs are not trusted by the RGB proxy client any more (on a wallet that is
  the safer default), and the roots move only with `webpki-roots` updates.
- **sea-orm** uses `runtime-tokio` instead of `runtime-tokio-rustls`: SQLite has no TLS.
- **Migration crate**: the CLI and Postgres moved behind a `cli` feature, on by default
  in the migration crate (so `sea-orm-cli migrate` keeps working there, see
  `migration/README.md`) and off in rgb-lib's dependency on it. The binary target
  requires `cli`.
- Two features the library used but never declared are now explicit: `hex/std`
  (arrived only through sqlx-postgres) and `tokio/rt-multi-thread` (only through
  sea-orm-cli). Without them the library does not compile once those crates are gone.

### Side effect: VSS over https works

Before this change the app's feature set enabled both `aws-lc-rs` (through reqwest) and
`ring` (through bitreq) on rustls 0.23. bitreq builds its TLS config with
`ClientConfig::builder()`, which cannot choose between two compiled-in providers, so
the first VSS request panicked with "Could not automatically determine the
process-level CryptoProvider". Reproduced on the macOS host against
`https://vss-server.utexo.com/vss` before the change (the Android graph enabled the
same two features); `Ok(None)` for the same read-only request after. The dependency
guard in `era.yml` is what keeps it that way: no aws-lc in the app graph means rustls
has exactly one provider. Electrum builds still compile both providers and still
depend on something installing a default first (rgb-lib does so when it builds an
Electrum indexer) — worth reporting to UTEXO separately.

### What remains, deliberately

- **ring** is the one crypto backend. minreq (behind `esplora-client 0.12`, pinned by
  `rgb-ops`) and bitreq (behind `vss-client`) hard-wire rustls on ring; aws-lc would
  have been a second backend, not a replacement.
- **Two rustls versions**, 0.21 (minreq) and 0.23 (everything else), both on ring.
  Unifying them needs an esplora-client without minreq under rgb-ops.
- **rustls-platform-verifier** is still compiled: reqwest's `rustls-no-provider`
  depends on it unconditionally. It is never called.
- **Electrum builds keep aws-lc**: `rgb-ops` depends on `electrum-client` with its
  default features (rustls on aws-lc). The app does not build `electrum`.

### Evidence (aarch64-linux-android, `--no-default-features --features esplora,vss`)

`cargo tree -e normal,build`:

| Crate | Before | After |
|---|---|---|
| openssl-sys / openssl-src / native-tls | 0.9.116 / 300.6.0+3.6.2 / 0.2.18 | — |
| aws-lc-rs / aws-lc-sys | 1.17.0 / 0.41.0 | — |
| ring | 0.17.14 | 0.17.14 |
| sqlx-postgres | 0.9.0 | — |
| clap / sea-orm-cli | 4.6.1 / 2.0.2 | — |
| rustls | 0.21.12, 0.23.40 | 0.21.12, 0.23.40 |
| rustls-platform-verifier | 0.7.0 | 0.7.0 (unused) |

The same holds for `armv7-linux-androideabi`, `x86_64-linux-android` and
`aarch64-apple-ios` (iOS had aws-lc, clap and sqlx-postgres, no OpenSSL); `era.yml`
fails if any of them comes back.

Release size, NDK r28c, API 31, `lto = "fat"`, `codegen-units = 1`,
`overflow-checks = true` (the app's profile), a cdylib exposing the fork's whole C-FFI
surface (`bindings/c-ffi/src`, so every online path is reachable and nothing is
dropped as dead code):

| `librgb_size_probe.so` (arm64-v8a) | Before | After | Change |
|---|---|---|---|
| as linked | 40.24 MB | 30.73 MB | −9.51 MB (−24 %) |
| `llvm-strip --strip-unneeded` | 33.00 MB | 25.11 MB | −7.89 MB (−24 %) |
| stripped, `gzip -9` (≈ download) | 16.14 MB | 12.43 MB | −3.71 MB |
| `.text` | 22.66 MB | 18.44 MB | −4.22 MB |
| OpenSSL / aws-lc / ring symbols | 172 / 1220 / 72 | 0 / 0 / 72 | |

LOAD segments stay 16 KB-aligned (`-z max-page-size=16384`, as cargokit links).

The app's current `libera_rgb.so` is smaller because its smoke API reaches little of
rgb-lib; the difference grows toward these numbers as the app exposes the online API.

## Carrying the series onto a new UTEXO tag

```sh
git fetch utexo --tags
git switch -c era/<name> <utexo-tag>                    # a new branch; never rebase this one
git cherry-pick 62a8c3a..era/configurable-derivation   # the whole series, in order
git push origin era/<name>                              # the branch only, never --tags
```

- `6ce375e` applies without conflicts on `v0.3.0-beta.34-bfa` and on
  `v0.3.0-beta.43-bfa` (checked with `git merge-tree`, 2026-09-26). It has not been
  compiled there: the `-bfa` tags `[patch.crates-io]` rgb-consensus / rgb-ops /
  rgb-schemas with private UTEXO repositories.
- `1d26404` conflicts on `v0.3.0-beta.43-bfa` in `release.yml`, which UTEXO changed.
  Keep their file and put the owner condition back on every job.
- The diet commit touches `Cargo.toml` and the three `Cargo.lock` files. On a
  conflict in a lock file, keep the new base's lock (`git checkout --ours <lock>`),
  then let Cargo prune it with `cargo tree -p rgb-lib > /dev/null` (and the same in
  `bindings/c-ffi`, `bindings/uniffi`). Do **not** run `cargo update`: a fresh
  resolution fails on the yanked secp256k1.
- Then run the steps of `era.yml` in order, both jobs (the `https` one needs network),
  with the commands exactly as written there. The workflow is the checklist; a list
  copied here would drift from it.
- If the HTTP client guard fires, route the new client through
  `api::rest_client_builder`; do not widen the exclusion.
- If UTEXO has merged the layout patch, drop `6ce375e` and check that their field
  names and defaults match what the app sends.
- In the app: bump the rev in `packages/era_rgb/rust/Cargo.toml`, copy this
  repository's `Cargo.lock` over the app crate's and run `cargo update --workspace`
  there (the comment next to the dependency explains why). The copy drops the app's
  own entries and `cargo update --workspace` resolves them afresh, so it can move
  crates rgb-lib never uses. Diff the app's old and new `Cargo.lock` and accept
  changes only in rgb-lib's graph (`cargo tree -p rgb-lib --target all -e normal,build
  --prefix none`); anything else goes back with
  `cargo update -p <crate>@<new> --precise <old>`. The other way round: keep the app's
  lock, change the rev, run `cargo update -p rgb-lib`, then compare every version in
  rgb-lib's graph with this repository's lock.

## PR proposal for UTEXO

Text only; nothing has been opened. The PR would carry `6ce375e` alone, rebased on
their current `dev`.

---

**Title:** Configurable keychain layout for singlesig keys (hardware-wallet hosts)

**Summary**

`SinglesigKeys` gains three optional fields — `colored_keychain`,
`colored_coin_type`, `vanilla_coin_type` — so a watch-only host can place the colored
and the vanilla side of a wallet under account paths its signer can actually export.
Unset, they resolve to today's defaults: descriptors, derivation paths and manifests
of existing wallets are unchanged.

**Motivation**

The colored side lives under the RGB coin type (`m/86'/827166'/0'`, `827167'` off
mainnet), so a watch-only wallet needs an account xpub at that path. Hardware
wallets export the standard accounts (BIP-44/49/84/86 at coin type `0'`/`1'`) and
cannot export `827166'` without a firmware release. We build the ERA hardware wallet
and its companion app; with this change the app can run an rgb-lib watch-only wallet
with both sides under the one account the device exports (`m/86'/0'/0'`, colored on
keychain 9, vanilla on keychain 10), so every input the device is asked to sign is on
a path it already derives. The same applies to any signer with a fixed set of
exportable accounts.

**Changes**

- `SinglesigKeys`: `colored_keychain: Option<u8>`, `colored_coin_type: Option<u32>`,
  `vanilla_coin_type: Option<u32>` (camelCase in the C-FFI JSON; numbers or numeric
  strings, like `vanilla_keychain`), plus `SinglesigKeys::with_keychain_layout`.
- `KeychainLayout::resolve` resolves the defaults per network and rejects a coin type
  that is not a valid hardened index, and a layout where both sides resolve to the same
  keychain (colored UTXOs would become spendable as vanilla ones).
- New `Error::InvalidKeychainLayout { details }` (also in the uniffi UDL).
- `get_descriptors` / `get_descriptors_from_xpubs` take the resolved layout.
- `WalletManifest` persists the overrides only when they differ from the default, so
  existing manifests stay byte-identical; a changed layout on `load` is a
  `WalletSettingMismatch`, like the other immutable settings.
- New `utils::get_account_data_at_coin_type`; `get_account_data` delegates to it.
- Multisig unchanged: cosigners share one descriptor built from the constants and
  would need to agree on a layout first.

**Compatibility**

- No behaviour change unless the new fields are set; `default_layout_descriptors_unchanged`
  pins the default descriptors.
- Additive API. The only source-level impact is for code that builds `SinglesigKeys`
  with a struct literal (three new fields); the constructors are unchanged.
- Interoperability: derivation paths never leave the wallet (invoices, consignments,
  ACKs and witness transactions do not carry them). On your signet we ran a
  custom-layout watch-only wallet against a default-layout wallet in both directions
  (a transfer to a blind invoice of the custom-layout wallet, then one from it to a
  witness invoice of the default wallet): both transfers settled on both sides,
  balances matched, and the PSBTs carried the custom paths in their key origins.

**Tests**

`cargo test --lib --features esplora,vss tests_keychain_layout` — default layout per
network, rejection of a shared keychain and of a non-hardenable coin type, the
single-account layout built from xpubs and from a mnemonic (identical descriptors),
unchanged default descriptors.

**Open questions**

- Would you rather expose a single `KeychainLayout` struct in the public API instead of
  three loose options?
- Is there interest in the same for multisig, where every cosigner would need to
  declare the layout?

---
