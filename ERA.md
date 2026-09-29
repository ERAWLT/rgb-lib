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
  condition (see [§2](#2-ci)). This clone
  holds UTEXO's tags from `git fetch utexo --tags`, including `v0.3.0-beta.43-bfa`,
  which origin does not have.
- A new UTEXO base gets a **new branch** (see [Carrying the series](#carrying-the-series-onto-a-new-utexo-tag)),
  never a rebase of this one.
- Commits are authored by people, with no AI co-author trailers.

## The series

| Commit | Subject | Upstream? |
|---|---|---|
| `6ce375e` | Add a configurable keychain layout to singlesig keys | Proposed to UTEXO: [UTEXO-Protocol/rgb-lib#104](https://github.com/UTEXO-Protocol/rgb-lib/pull/104) (2026-09-28, text [below](#pr-proposal-for-utexo)) |
| `1d26404` | Run the fork's own checks on era branches only | Fork-only |
| `f808c7f` | Carry one TLS stack and no migration CLI in the library | Fork-only |
| `d82e21a` | Document the ERA fork of rgb-lib (this file) | Fork-only |
| `07e16e5` | Guard HTTP client construction and probe real https in CI | Fork-only |
| `e7bdaa4` | Route RGB proxy traffic through an optional loopback forwarder | Fork-only (generic enough to offer UTEXO later; nothing drafted) |
| `3f0a555` | Put the forwarder comment on the forwarder test module | Fork-only |
| `fafe3bd` | Fail closed when a forwarded reject list is not a 2xx answer | Fork-only |
| `bef0c03` | Route every proxy call site through the wallet's own helpers | Fork-only |
| `ccd23e9` | Report a forwarder's refusal as Error::ForwarderRefused | Fork-only |
| `8a0255f` | Refuse forward targets that carry userinfo or a fragment | Fork-only |
| `cf0afa5` | Apply a new forwarder before go_online probes a new indexer | Fork-only (superseded by `ff6e73a`) |
| `9718cac` | Keep the forwarder's URL out of request errors | Fork-only |
| `0996972` | Describe the reviewed forwarder contract in its rustdoc | Fork-only |
| `29a49bb` | Run the forwarder tests one at a time | Fork-only |
| `ff6e73a` | Go offline when go_online fails on a forwarder's new indexer | Fork-only |
| `afe4644` | Accept a refusal only when it echoes the forwarder's path | Fork-only |
| `aa5187d` | Match "Cannot change ACK" on the proxy's error only | Fork-only |
| `a14cdbb` | Let a send the forwarder refuses be failed, and not block others | Fork-only |
| `b8bf73d` | Read a forwarded reject list only from a 200 that holds an opout | Fork-only |
| `019b917` | Test what the review's surviving mutations left unchecked | Fork-only |
| `87d5e88` | Guard check_proxy_url calls and build the bindings in CI | Fork-only |
| `45c3b39` | Keep the VSS URL and store ID out of the restore log | To propose to UTEXO |
| `777fe57` | Check the VSS server's word before a restore acts on it | To propose to UTEXO (the upstream half) |
| `fe7e1b0` | Fail a refused send only if the indexer does not know its TX | Fork-only |
| `1051adc` | Keep an expired send whose TX is on chain from failing on its ACKs | To propose to UTEXO (their own path, forwarder or not) |
| `1699a6a` | Undo a skipped transfer's attempt when failing every expired one | Fork-only (the savepoint would suit UTEXO's own skip on the `-bfa` tags) |
| `5a0a980` | Look for "Cannot change ACK" in the NACK's answer, not in an Err | To propose to UTEXO (their arm never matched either) |
| `0b14aae` | Go offline when go_online refuses a URL replacing a forwarder | Fork-only |
| `f62e768` | Require a session path in forwarder_url and log unechoed refusals | Fork-only |
| `ba77828` | Read a forwarded reject list only from a body with a checked end | Fork-only |
| `dcc9654` | Refuse backup data that ends without its final block | To propose to UTEXO |
| `55cb0c9` | Keep a VSS server's numbers from crashing or sizing a restore | To propose to UTEXO |
| `b8df12d` | Restore only the wallet directory of a VSS backup, and only whole | To propose to UTEXO |
| `6921d07` | Refuse a plaintext VSS backup whenever encryption is on | To propose to UTEXO (it changes what a default config restores) |
| `bb85811` | Leave nothing behind a VSS restore that fails | To propose to UTEXO |
| `d6357cf` | Deprecate restore_from_vss in favour of the expecting variant | Fork-only |
| `8692b69` | Catch more ways around the grep guards in era.yml | Fork-only |
| `56e6186` | Never fail an outgoing transfer whose TX the indexer knows | To propose to UTEXO (the chain check; the refusal half is fork-only) |
| `4ea8ccd` | Match a backup's fingerprint ignoring case, and keep its name | To propose to UTEXO (with `b8df12d`) |
| `c188e63` | Remove only what a failed VSS restore made itself | To propose to UTEXO (with `bb85811`) |
| `ba96a80` | Bound a VSS manifest's chunks by the chunk size, decrypt as it arrives | To propose to UTEXO |
| `3988f38` | Stop claiming an interrupted chunked upload keeps the previous backup | To propose to UTEXO |
| `c88f20a` | Leave the forwarder's path out of its Debug form | Fork-only |
| `a140543` | Guard every mention of the proxy and reject-list client types | Fork-only |
| `b761c25` | Sync a restored wallet before renaming it into place, and its parent | To propose to UTEXO |
| `5cab571` | Split the record half out of broadcast_psbt | To propose to UTEXO (with `c1feef6`) |
| `f82b1c1` | Add a scripted chain for offline wallet tests | Fork-only (could serve UTEXO's own offline tests) |
| `27d2a5a` | Refuse a signed.psbt whose txid is not its transfer's | To propose to UTEXO |
| `c1feef6` | Complete an own spend the indexer knows at go_online (CC-99) | To propose to UTEXO, opt-in and default off: their `consume_transfer_fascia` (`-bfa` `rust_only.rs`) completes the same class of spend under the same bar, but needs the `Online` handle this state never grants |
| `4093572` | Let the host tell a recorded vanilla TX from an unknown one | To propose to UTEXO (with `c1feef6`) |
| `106a00c` | Reserve the external-operation reason for the -bfa carry | Fork-only (goes with `c1feef6`) |
| `ac6724d` | Store the RGB stock atomically, and report a store the disk refused | To propose to UTEXO (every rgb-lib wallet) |
| `7d07452` | Load the RGB stock only as a whole set | To propose to UTEXO (with `ac6724d`) |
| `74664d9` | Clear the completion report before go_online can fail | To propose to UTEXO (with `c1feef6`) |
| `3768cc6` | Keep a failed lookup's text out of the completion's log line | Fork-only (the secret is the forwarder's) |
| `3b856ac` | Test the completion against a stash the disk will not take | To propose to UTEXO (with `c1feef6`) |
| `adb712a` | Pin what a stash refusal keeps of the spends before it | To propose to UTEXO (with `c1feef6`) |
| `fe22585` | Pin P1 with another install's spend of the same coin | To propose to UTEXO (with `c1feef6`) |
| `2b44c76` | Pin offline that the option off is upstream's go_online | To propose to UTEXO (with `c1feef6`) |
| `690f041` | Test a retryable I/O error and the checks after a completion | To propose to UTEXO (with `c1feef6`) |
| `5f5edb8` | Hold the stash back while the index or the state is not stored | To propose to UTEXO (with `ac6724d`) |
| `5fdfa17` | Count a bundle as held only when the index knows it too | To propose to UTEXO (with `c1feef6`) |
| `1a078a7` | Sync rgb/ when a store's directory sync was missed, and at load | To propose to UTEXO (with `ac6724d`) |
| `aee685d` | Say what RgbStockDamaged is: a refusal to open, not a wipe signal | To propose to UTEXO (with `7d07452`) |
| `5dd72f3` | Attribute a restored wallet's unopenable stock to the VSS backup | To propose to UTEXO (with `7d07452`) |
| `f82cc6b` | Pin a stock an older rev wrote, and one over 64 KiB | To propose to UTEXO (with `ac6724d`) |
| `1ee1caf` | Let a send whose signed.psbt will not be broadcast be failed | To propose to UTEXO (with `27d2a5a`; the unparsable half is their own lock) |
| `74a2d66` | Test go_online's asset check again, offline and upstream | To propose to UTEXO (with `7d07452`) |
| `50a470b` | Keep the kind when a restored wallet's stock cannot be opened | To propose to UTEXO (with `7d07452`) |
| `3889751` | Say a linked rgb is a symbolic link, and touch nothing behind it | To propose to UTEXO (with `7d07452`) |
| `f5d6522` | Pin the undecodable kind and the kinds' codes | To propose to UTEXO (with `7d07452`) |
| `df04de0` | Stop promising in rustdoc that a refused completion writes nothing | To propose to UTEXO (with `c1feef6`) |
| `d326033` | Build reqwest without HTTP/2 | Fork-only (a size choice for the app; UTEXO's other hosts may want HTTP/2) |
| `1ed4436` | Order InvalidKeychainLayout and merge the SinglesigKeys impls | Review of #104, mirrored (PR branch `c23b140`) |
| `2319444` | Group the keychain layout overrides into one struct | Review of #104, mirrored (PR branch `eba4d69`) |
| `e230950` | Refuse account xpubs that contradict the configured coin types | Review of #104, mirrored (PR branch `c1a1ee8`) |

Later commits that touch only this file are part of the series too. `3f0a555` to `87d5e88`,
`fe7e1b0` to `ba77828`, `8692b69`, `56e6186`, `c88f20a` and `a140543` answer four reviews of
`e7bdaa4` ([§4](#4-proxy-forwarder-e7bdaa4)) and go with it wherever the series is carried;
`45c3b39`, `777fe57`, `dcc9654` to `d6357cf`, `4ea8ccd` to `3988f38` and `b761c25` are
[§5](#5-backup-restore-checks); `5cab571` to `106a00c` and, after their review, `74664d9` to
`690f041` and, after their verification, `1ee1caf` and `df04de0` are
[§6](#6-completing-an-own-unrecorded-spend-cc-99); `ac6724d`, `7d07452`, after their review
`5f5edb8` to `f82cc6b`, and after its verification `74a2d66` to `f5d6522` are
[§7](#7-the-rgb-stock-on-disk-cc-101); `d326033` extends [§3](#3-dependency-diet-f808c7f);
`1ed4436` to `e230950` answer UTEXO's review of #104 and go with `6ce375e`
([§1](#1-configurable-keychain-layout-6ce375e)).

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

- `SinglesigKeys` (`src/wallet/singlesig.rs`) gets a `keychain_layout:
  KeychainLayoutOverrides` field next to `vanilla_keychain`, holding three optional
  settings: `colored_keychain: Option<u8>`, `colored_coin_type: Option<u32>`,
  `vanilla_coin_type: Option<u32>` (three loose fields until the review of #104,
  `2319444`). The struct is `#[serde(flatten)]`ed, so in JSON / C-FFI the three stay
  top-level keys next to `vanilla_keychain` (`coloredKeychain`, `coloredCoinType`,
  `vanillaCoinType` with `camel_case`) and accept a number or a numeric string like the
  existing fields. `SinglesigKeys::with_keychain_layout(KeychainLayoutOverrides)` sets
  them on keys built by `from_keys` / `from_keys_no_mnemonic`. `vanilla_keychain` stays
  a field of `SinglesigKeys`; in the UDL the struct is a dictionary of its own.
- `KeychainLayout::resolve` (`src/utils.rs`) turns those options into concrete coin
  types and keychains for the wallet's network and validates them: a coin type must
  be a valid hardened index (`< 2^31`), and the colored and vanilla sides must not
  resolve to the same (coin type, keychain) pair, because one BDK keychain feeding
  both sides would make colored UTXOs spendable as vanilla ones.
- `build_descriptors` requires the two account xpubs to be the same key exactly when the
  two coin types are equal (`e230950`, from the review of #104): an xpub under a coin
  type it was not derived at still receives, but its PSBTs carry origins the signer
  will not derive. It compares public key and chain code, so an `xpub`/`tpub` of one key
  and a key rebuilt with other depth or parent fingerprint count as the same key.
- `Error::InvalidKeychainLayout { details }` reports a rejected layout (also added to
  the uniffi UDL).
- `get_descriptors` / `get_descriptors_from_xpubs` build both descriptors from the
  resolved layout instead of the constants; the key origin in the descriptor (and so
  in every PSBT) is the layout's path.
- `wallet_manifest.json` stores `colored_keychain` / `colored_coin_type` /
  `vanilla_coin_type` **only when they differ from the default** (serde
  `skip_serializing_if`), so manifests of default-layout wallets stay byte-identical;
  a layout that changes between `new` and `load` is a `WalletSettingMismatch`. The
  manifest keeps these as its own flat snake_case fields rather than flattening
  `KeychainLayoutOverrides`, which `camel_case` would rename.
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

From `2319444` the app passes the three through `keychain_layout: KeychainLayoutOverrides
{ colored_keychain: Some(9), colored_coin_type: Some(0), vanilla_coin_type: Some(0) }`,
with `vanilla_keychain: Some(10)` where it was. From `e230950` the one account xpub has to
be passed as both (it always is): two different keys under one coin type are refused as
`InvalidKeychainLayout`.

### Tests

`cargo test --locked --lib --features esplora,vss -- tests_keychain_layout` (9 tests:
defaults per network, shared-keychain and non-hardenable rejections, the single-account
layout from xpubs and from a mnemonic, unchanged default descriptors; since the review of
#104 also the xpub/coin-type check in both directions, and `SinglesigKeys` JSON and
manifests as `6ce375e` wrote them, read into the struct and written back byte for byte).
`wallet::test::load::keychain_layout_roundtrip_success` loads a custom layout back from
the manifest and refuses another.

## 2. CI

`.github/workflows/era.yml` (`1d26404`, extended in `f808c7f`, `07e16e5`, `e7bdaa4`, `bef0c03`,
`87d5e88`, `45c3b39`, `dcc9654`, `8692b69`, `a140543`, `f82b1c1`, `27d2a5a` and `d326033`) has two jobs.

- **check** runs on push / PR to `era/**` and on manual dispatch: rustfmt, `cargo check`
  with the app's feature set and with the upstream default on top, a check of the uniffi
  and C-FFI bindings (each its own workspace and lockfile; the uniffi UDL mirrors the `Error`
  enum, and a variant missing there breaks only that build), a dependency guard over the
  app's mobile targets, the HTTP client guard (below), the proxy forwarder guard
  ([§4](#4-proxy-forwarder-e7bdaa4)), the unit tests above plus the REST-client TLS test, the
  forwarder tests, the VSS and file backup tests ([§5](#5-backup-restore-checks)), offline
  wallet tests, the scripted-chain and CC-99 tests
  ([§6](#6-completing-an-own-unrecorded-spend-cc-99)), the stock store tests
  ([§7](#7-the-rgb-stock-on-disk-cc-101)), and the migration crate. Every cargo call is `--locked`: a fresh resolution
  fails on the yanked `secp256k1 0.32.0-beta.2` that `rgb-consensus 0.11.1-rc.11` requires.
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
fails the handshake unless the app initialised it over JNI. The step fails when a
construction appears in `src/` outside `src/api/mod.rs`: by its path
(`RestClient::new/builder/default`, `reqwest::(blocking::)Client::` / `ClientBuilder::`,
`blocking::Client::`), bare (`Client::new`, `Client::builder`, `ClientBuilder::new`, what
`use reqwest::blocking::Client;` leads to, since `8692b69`), or reqwest's `get`, called or
imported; and when the pattern stops matching the one construction there. A field of type
`reqwest::blocking::Client` and `lib.rs`'s aliased import are not constructions and do not
match. It is a grep, not a type check: an alias (`use reqwest::blocking::Client as Http;
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
the Esplora indexer and VSS over https directly (and the RGB proxy stays on https
whenever `forwarder_url` is unset, [§4](#4-proxy-forwarder-e7bdaa4)).

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
- **HTTP/1.1 only** (`d326033`, from the app's size pass T4.8, 2026-09-29): reqwest is built
  without `http2`, so h2 and hyper's HTTP/2 client are gone from the app's graph (0.28 MB of
  the Android arm64 library at the app's opt-level `"s"`). The app does not lose anything by
  it: with a forwarder ([§4](#4-proxy-forwarder-e7bdaa4)), which every release build of the
  app has, each RGB proxy and reject-list request is plain HTTP/1.1 to loopback, which
  reqwest never upgrades; the direct https route negotiates HTTP/1.1, which RGB proxies
  serve. `rest_client_builder` builds the TLS config itself, so its ALPN is kept in step by
  hand: `http/1.1` alone. Offering `h2` to a client that cannot speak it would let a server
  pick it, and hyper-util then panics in reqwest's connection task ("http2 feature is not
  enabled", `client/legacy/client.rs`) instead of sending the request;
  `tests_rest_client_tls::rest_clients_offer_only_http1_in_alpn` pins the list, and the
  dependency guard in `era.yml` bans `h2` from the mobile graphs. `h2` stays in the root
  `Cargo.lock` because mockito (a dev-dependency) needs hyper's HTTP/2 server; that
  unification reaches `cargo test` builds only. UTEXO's signet RGB proxy does not offer h2
  at all (it answers HTTP/1.1 to a client offering h2, `curl --http2`, 2026-09-29), so the
  direct route to it was HTTP/1.1 before this change too, and the https test cannot notice
  an `h2` in the ALPN: a unit test pins it.
  Checked against UTEXO's `-bfa` tags with `git merge-tree` as the rest of the series
  (2026-09-29): no conflict region beyond those the series already has.

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
  Unifying them needs an esplora-client without minreq under rgb-ops, which enables
  `esplora-client/blocking-https` itself (0.21 with its webpki is about 0.14 MB of the
  Android arm64 library at the app's opt-level `"s"`).
- **rustls-platform-verifier** is still compiled: reqwest's `rustls-no-provider`
  depends on it unconditionally, and reqwest's builder references it in a branch our
  prebuilt config never takes, so the linker keeps it. It is never called. On Android it
  brings the `jni` crate: about 0.08 MB of the arm64 library at opt-level `"s"` (0.12 MB
  at 3). Dropping it means patching reqwest.
- **reqwest `json` and `multipart`** are used: the RGB proxy is JSON-RPC
  (`ProxyClient`), and `consignment.post` / `media.post` upload multipart forms.
- **reqwest `charset`** stays (encoding_rs: a build without it was 0.18 MB smaller on
  Android arm64 at `"s"`, 2026-09-29). Without it `Response::text()` is `from_utf8_lossy`: a reject list
  served with a UTF-8 BOM would keep it on its first line, which would then not parse as
  an opout and be skipped, and the list would lose that entry silently. Keeping the
  decoder is cheaper than carrying a decoder of our own on that path.
- **zip with zstd** stays: backups (`backup.rs`, `vss.rs`) are written with
  `CompressionMethod::Zstd`, so reading an existing backup needs it; zstd is already
  built without its default features (no legacy formats, no dictionary builder).
- **Electrum builds keep aws-lc**: `rgb-ops` depends on `electrum-client` with its
  default features (rustls on aws-lc). The app does not build `electrum`.
- **The app's own size settings are not here.** Cargo applies profiles only from the
  top-level crate, so the optimisation level and the `lowmemory` switch of both
  libsecp256k1 copies in this graph (2 MB of precomputed tables) are set in the app's
  `packages/era_rgb/rust/Cargo.toml` (T4.8).

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

## 4. Proxy forwarder (`e7bdaa4`)

Reworked after three independent reviews on 2026-09-27, all "pass with issues". The first:
`fafe3bd` (reject list fails closed), `bef0c03` (the route comes only from the wallet),
`ccd23e9` (refusal signal), `8a0255f` (no userinfo or fragment in a target), `cf0afa5`
(forwarder applied before the indexer probe, superseded by `ff6e73a`), `9718cac` (errors never
name the forwarder), `0996972` (rustdoc). The second: `ff6e73a` (a failed `go_online` goes
offline; `go_offline`), `afe4644` (a refusal must echo the forwarder's path), `aa5187d` ("Cannot
change ACK" from the proxy only), `a14cdbb` (a refused send can be failed and does not block
others), `b8bf73d` (a forwarded reject list only from a 200 that holds an opout), `019b917`
and `87d5e88` (tests and CI for what the second review's mutations and probes left open), with
`29a49bb` running the forwarder tests one at a time. The third: `fe7e1b0` and `1051adc` (a send
is failed only if its TX is not on chain), `1699a6a` (a skipped transfer keeps none of its
attempt's database writes), `5a0a980` ("Cannot change ACK" where the proxy puts it), `0b14aae` (a refused forwarder
URL goes offline), `f62e768` (a path is required, an unechoed refusal is logged), `ba77828` (a
forwarded reject list only whole) and `8692b69` (wider guards). The fourth: `56e6186` (no
outgoing transfer is failed while the indexer knows its TX, whatever path leads there),
`c88f20a` (the forwarder's Debug form without its path) and `a140543` (the proxy guard lists every
mention of the client types). This section describes the result.

### Why

The app sends all of rgb-lib's network traffic through a forwarder it runs on `127.0.0.1`
(the plan's D7 = N2, task T1.8), which decides for each request whether and how it leaves the
phone ([below](#the-forwarders-side)). The Esplora indexer and VSS take whatever URL the host
passes, so they can point at the forwarder. The RGB proxy cannot: its endpoint is a property of
each **invoice**. The receiver polls the endpoints in its own invoice, the sender posts to the
endpoints in the receiver's invoice (`online.rs` `wait_consignment` / `post_transfer_data`). An
invoice carrying `rpc://127.0.0.1:<port>` is useless to the other wallet, and one carrying the
public endpoint makes rgb-lib contact the proxy directly, around the forwarder (CC-79 in the
app's docs).

The reject list has the same shape, one step worse: its URL comes from the asset contract
(`reject_list_url`), and consignment validation in `refresh`, `send_begin` and
`provide_out_of_band_consignment` fetch it. Any asset sent to the wallet can name a host rgb-lib
will then contact. So the patch routes it too.

### What the app calls

```rust
// rgb_lib::wallet::OnlineOptions gains one field (`forwarderUrl` with the camel_case feature)
let online = wallet.go_online(OnlineOptions {
    indexer_url,                        // the forwarder's Esplora route (T1.8)
    skip_consistency_check: false,
    vanilla_sync_lookback,
    // None = upstream behaviour; the path (required) carries the session's secret
    forwarder_url: Some(format!("http://127.0.0.1:{port}/{session_secret}/rgb")),
})?;
// ... and when the session ends (lock, pause): no proxy, reject-list or indexer request until
// the next go_online (VSS is configured apart, see below)
wallet.go_offline();

// the forwarded twin of rust_only::check_proxy_url (not needed by a wallet that is online)
rgb_lib::wallet::rust_only::check_proxy_url_via_forwarder(proxy_url, forwarder_url)?;
```

- `forwarder_url` must be plain `http` to a loopback IP literal (`127.0.0.0/8` or
  `[::1]`; `localhost` is refused), **with a path** (not `/`), and with no credentials, query,
  fragment or port 0. The path is used as given. Anything else is
  `Error::InvalidForwarderUrl { details }`, raised by `go_online` before it changes anything:
  a wallet no forwarder routes stays as it was, one a forwarder routes goes offline (below).
- **The path carries a per-session secret, and is required** (`f62e768`). A loopback port is
  reachable by every app on the phone, so the forwarder cannot tell rgb-lib from another app by
  the connection alone. A random path segment per session (`/{session_secret}/rgb`), checked by
  the forwarder on every request, can; it also authenticates the forwarder's refusals
  ([below](#what-rgb-lib-guarantees)), which is why a URL without one is refused: its refusals
  could not be told from a relayed target's.
  rgb-lib keeps `forwarder_url` in memory only (the wallet's `OnlineData`): it is not written to
  the wallet's files, invoices, the database or a backup, not logged, a failed request's error
  names the URL the request was meant for, never the forwarder's (`9718cac`), and the
  forwarder's `Debug` form names its origin only (`c88f20a`).
  `OnlineOptions` derives `Debug` and `Serialize`, so its debug output and its JSON carry the
  secret: **the host must not log it**.
- **`go_online` applies the value on every call; a call that fails at the probe of a new
  indexer goes offline.** A refused `forwarder_url` changes nothing when no forwarder is set,
  and goes offline when one is (`0b14aae`), for the reason that follows. Otherwise the forwarder
  follows the call. When the call changes `indexer_url` and the probe of the new URL fails,
  and a forwarder is set before or by the call, the wallet drops its online state instead of
  staying online on the previous indexer as upstream does (`ff6e73a`): in the app the previous
  indexer URL is the forwarder's Esplora route of the previous session, the previous port and
  secret, which another app may own by now. Every `Online` handle then gets `Error::Offline`
  until the next successful `go_online`, which returns a new one. Without a forwarder on either
  side, upstream's behaviour stays. A first `go_online` that fails leaves the wallet offline, as
  it was.
- **`Wallet::go_offline()`** (fork-only, `ff6e73a`) drops the online state on purpose: the
  indexer and resolver clients and the route. The app calls it when a session ends (lock,
  pause), since the forwarder's port and secret end with the session. Going offline twice is not
  an error. Two things it does not do:
  - **It does not interrupt a call.** It takes `&mut self`, so behind the host's lock it waits
    for a call already running, and rgb-lib cannot cancel one: left alone it runs to its
    client's timeout (120 s for a proxy request; 10 s per indexer or VSS request, each retried
    as its client does). So **before calling `go_offline` the host has the forwarder fail every
    request of the session still in flight** (drop the connection, or answer as for its own
    failures, [below](#the-forwarders-side)), and the forwarder keeps the port bound, failing
    new requests the same way, until `go_offline` has returned.
  - **It does not touch VSS.** The VSS client (`configure_vss_backup`) is not online state: an
    auto-backup already uploading goes on, and an operation that changes the wallet while offline
    can start one, to the server URL the client was configured with. A host whose VSS route ends
    with the session reconfigures VSS, or calls `disable_vss_auto_backup`, with it.
- The setting lives in `OnlineOptions` rather than behind a wallet setter because it is a
  route like `indexer_url`: both change in the same call. Every proxy request happens inside
  an online method, after the `Online` check, so the forwarder is always in reach.
- A missing field deserializes to `None` (C-FFI JSON), the uniffi dictionary defaults it to
  `null`. A Rust struct literal has to name it: that is deliberate, a host upgrading the rev
  has to decide.

### What rgb-lib guarantees

With `forwarder_url` set, for every request rgb-lib makes to an RGB proxy or a reject list:

- **Destination**: `forwarder_url` exactly (scheme, host, port and path as given, nothing
  appended), HTTP/1.1 over plain TCP. Redirects are not followed (a 3xx is read as an
  answer), the system proxy is ignored (`no_proxy`, so an `HTTP_PROXY` in the environment is
  not used; a test runs with one set), and no error of any kind leads to a direct connection.
  Wallet code gets these clients only from the wallet's own route
  ([Guard and tests](#guard-and-tests)). Timeouts: 10 s to connect, 120 s for the whole request.
- **Headers added**, and no others:
  - `X-Era-Forward-Target`: the absolute URL rgb-lib would have requested, as `url::Url`
    serializes it: scheme and host lower-case, a non-ASCII host in punycode, an IPv4 address
    in dotted decimal, the scheme's default port dropped, dot segments resolved, a character
    a path may not hold (a space) percent-encoded. It is not normalized further: a trailing dot
    in the host stays (`rgb-proxy.utexo.com.`), and percent-encoded bytes in the path stay
    encoded (`/json-rpc/..%2f..%2fadmin` is sent as it is). An `rpcs://host/path` transport
    endpoint becomes `https://host/path`, `rpc://` becomes `http://` (upstream's
    `TransportEndpoint::try_from`). Always `http` or `https` with a host (anything else fails
    inside rgb-lib, as it does without a forwarder). Never userinfo, never a fragment: a URL
    carrying either is `Error::InvalidForwardTarget` and is not requested at all (`8a0255f`), as
    `rpcs://rgb-proxy.utexo.com@evil.example/json-rpc` (host `evil.example`) would otherwise
    be. It may carry a query: `send_begin` probes an invoice endpoint with its recipient nonce
    (`?rid_nonce=<hex>`).
  - `X-Era-Forward-Kind`: `rgb-proxy` or `reject-list`.
- **Everything else as without a forwarder**: method, body, `Content-Type`.
  - `rgb-proxy`: always `POST`. `server.info`, `ack.get`, `ack.post`, `consignment.get`,
    `media.get` are `application/json` JSON-RPC bodies; `consignment.post` and `media.post`
    are `multipart/form-data` with the fields `method`, `jsonrpc`, `id`, `params` and the file
    part `file` (a consignment can be megabytes: stream it, keep the boundary).
  - `reject-list`: `GET`, no body; the answer is plain text, one opout per line.
- **Answers** are read as the target's own, with two exceptions:
  - **the refusal signal**: status **403** with a response header **`X-Era-Forward-Refused`**
    and a response header **`X-Era-Forward-Session`** whose value is exactly the path of
    `forwarder_url` (as rgb-lib requested it, e.g. `/6c1f0e…/rgb`). That is the forwarder
    refusing the request. The value of `X-Era-Forward-Refused` is the reason, passed through
    (up to 256 characters, decoded lossily) in `Error::ForwarderRefused { target, reason }`,
    `target` being the `X-Era-Forward-Target` value (`ccd23e9`). Anything less is an ordinary
    answer: a 403 without the refusal header, the header on another status, or a missing or
    different session header (`afe4644`). The echo is what a relayed target cannot produce: it
    never sees the path. A refusal header without a valid echo is logged as a warning in the
    wallet's log, without the path or the echoed value (`f62e768`): it comes from a forwarder
    that does not echo its path or relays a target's headers, and either turns every refusal
    into what reads as an outage.
  - **the reject list fails closed**: only a **200** is read as the list, only when its body
    has an end the client checks (a `Content-Length`, chunked as the last transfer coding, or
    HTTP/2), and a body that is not empty must hold at least one line that parses as an opout
    (plain or `!` allow line); anything else is `Error::RejectListService` (`fafe3bd`,
    `b8bf73d`, `ba77828`). Read as a list, an error page (or a 203, 204 or 206) holds no opout,
    and the asset would be validated against an empty list; a body the connection's close ends
    may be cut after its first line and read as a one-opout list. An empty 200 with such an end
    is an empty list: an issuer may publish one.

  For `rgb-proxy`, any other answer is read as JSON-RPC whatever its status, exactly as rgb-lib
  reads the proxy's own; a body that is not a JSON-RPC response is `Error::Proxy`. A reason is
  never read as the proxy's words: `refuse_consignment` looks for the proxy's "Cannot change
  ACK" in the JSON-RPC error of the NACK's answer, where the proxy puts it (`5a0a980`; upstream
  looked in the text of an `Err`, where it never arrives, and so did `aa5187d`), and a refusal
  returns its error whatever its reason says.

### How the forwarder must match a target

The allowlist decision is the forwarder's, on a URL that comes from a counterparty's invoice or
an asset contract. The rule:

1. Parse `X-Era-Forward-Target` as a URL, with a WHATWG-conformant parser (the form above is
   `url::Url`'s, so parsing and serializing it again gives the same string). Refuse the
   request if it does not parse, is not absolute `http` or `https`, or carries userinfo or a
   fragment: rgb-lib never sends those, so such a header is not rgb-lib's.
2. Compare **scheme**, **host** and **effective port** (the explicit port, else 443 for
   `https` and 80 for `http`) exactly with the allowlist entry, on the parsed values: the host
   as serialized (lower-case, punycode, dotted-decimal IPv4, bracketed IPv6). A trailing dot
   makes another host for this comparison (`rgb-proxy.utexo.com.` is not
   `rgb-proxy.utexo.com`): refused unless listed as such.
3. Never match on the header text: no prefix, suffix or substring test, no `startsWith`
   (`https://rgb-proxy.utexo.com@evil.example/` starts with a known host), no wildcard beyond
   what an entry says explicitly.
4. The **path** is per allowlist entry (for example exactly `/json-rpc` for a proxy, the
   list's own path for a reject list), compared as it arrives, still percent-encoded. Neither
   the forwarder nor the backend decodes it before matching or relaying:
   `/json-rpc/..%2f..%2fadmin` decoded and resolved is `/admin`.
5. The **query** is not part of the match (a probe carries `rid_nonce`); relay it as it is.
6. An entry holds for one **kind**: a proxy entry does not admit a `reject-list` request, nor
   the other way round.

### The forwarder's side

Not rgb-lib's code; what the fork's contract expects of it, consistent with the app's
decision D7 as amended by the project lead on 2026-09-27:

- **Check the caller**: the path of `forwarder_url` must carry the session's secret; any other
  path is refused.
- **Strip** every `X-Era-Forward-*` header: from the request before relaying (the `Host` is
  the target's), and **from every relayed response**. The forwarder sets `X-Era-Forward-Refused`
  and `X-Era-Forward-Session` itself, and only on its own refusals; a relayed response carrying
  them would let the target speak for the forwarder.
- **A target on the allowlist** (per kind) is relayed to the app's own **backend**, which
  relays it to the target (N2: pinning, the backend's own controls).
- **An unknown recipient proxy** (`rgb-proxy` only) may be reached **directly by the
  forwarder itself, only after the user explicitly approved a warning** naming that host:
  system TLS, redirects not followed, no pinning (there is nothing to pin for an arbitrary
  host). **The consent covers the transfer's whole life**, across sessions (kept per wallet and
  host until every transfer to that proxy is settled or failed): the probe (`server.info` in
  `send_begin`), the post (`consignment.post`, `media.post` in `send_end`) and **every**
  `ack.get` poll in `refresh`, which may come days later. A consent that lapses while a send
  waits for its ACK stops that send: its refresh fails with `ForwarderRefused` on each poll, and
  it completes only once the consent is back; it can be failed meanwhile (`a14cdbb`).
- **A reject list is allowlist-only**: there is no user to ask. The forwarder and the backend
  **never synthesize or cache a 2xx** for it: the answer is the issuer's current list or an
  error. Redirects are not followed, by the forwarder or by rgb-lib, so an issuer that moves its
  list behind a redirect makes the asset's transfers fail validation (`RejectListService` on
  every refresh) until the allowlist names the new URL; upstream's direct client follows up to
  ten.
- **`consignment.get` is not to be refused for this wallet's own invoices**: rgb-lib reads its
  refusal, like any failure there, as "no consignment yet" and shows nothing, so a receive on an
  unexpired invoice would wait for nothing. The wallet's invoices name the app's own proxy,
  which is on the allowlist; the forwarder logs any refusal of a `consignment.get`, since
  nothing else will.
- **Everything else is refused with the refusal signal**: `403`,
  `X-Era-Forward-Refused: <reason>` and `X-Era-Forward-Session: <the request's path>`, on every
  refusal: rgb-lib takes a refusal without the echo for an ordinary answer, and logs it. The
  reason is a short token of the app's choosing (for example `not-allowlisted`,
  `user-declined`, `consent-expired`) that the app maps to its own message; it is never shown
  raw.
- **Relay the target's status and body as they came, framed.** Send a relayed body with a
  `Content-Length` or chunked, never ended by closing the connection, and when the upstream body
  is cut short, abort the response (reset the connection, or end a chunked body without its last
  chunk) rather than finish it: rgb-lib then sees an error, not a shorter body.
- **Never answer with, follow or relay a 3xx, on every route**: the Esplora route, VSS, the RGB
  proxy and the reject list alike, in the forwarder and in any backend relay. An upstream 3xx
  becomes a `502`. rgb-lib's own proxy and reject-list clients do not follow a redirect through
  the forwarder, but its Esplora client (esplora-client over minreq) and its VSS client
  (vss-client-ng over bitreq) follow 301, 302, 303 and 307 to any host, resolved through the
  system's DNS, leaving the forwarder behind: in the review of `era_rgb` a 302 moved 44 indexer
  requests of one call to another host, and a 307 re-sent a signed VSS POST, its `Authorization`
  header included, to another host. rgb-lib cannot switch that off in either client
  ([below](#what-does-not-change-1)); this rule is what holds instead.
- **Its own failures** (the backend is down, the target cannot be reached, the session is
  suspended or ending) are answered with neither a 2xx nor the refusal headers (never a 200: for
  a reject list a 200 is the list), within rgb-lib's timeouts, and with a status that suits each
  route's client:
  - **Esplora route: `502`, or drop the connection.** esplora-client retries a 429, 500 or 503
    (three more times with rgb-lib's settings, waiting 256, 512 and 1024 ms: about 1.8 s per
    request), and fails at once on a 502, any other status or a broken connection. Timeout: 10 s
    per request.
  - **VSS route: `502`, or drop the connection.** vss-client-ng retries every error whatever
    the status (three attempts, 100 and 200 ms apart, within 5 s), so a dropped connection is the
    quickest failure. Timeout: 10 s per request.
  - **RGB proxy: `502`** (any non-2xx without the refusal headers): reqwest does not retry, and
    rgb-lib reads it as a proxy that is down, which a later call can retry. Timeouts: 10 s to
    connect, 120 s per request.
  - **Reject list: `502`** (any non-200): `RejectListService`. Timeouts as for the proxy.
- **Keep the previous session's port bound until `go_offline` has returned**, failing its
  requests as above; before the host calls `go_offline`, fail every request of the session still
  in flight ([above](#what-the-app-calls)): rgb-lib cannot cancel a call, and `go_offline` waits
  for it.
- **Relay the Esplora route's TX lookups as the indexer answers them**: an unknown TX gets
  `200 {"confirmed":false}` on `GET /tx/<txid>/status` and `404` on `GET /tx/<txid>/raw`, as
  electrs answers. rgb-lib looks an outgoing transfer's TX up before it fails it
  ([below](#how-it-surfaces-in-rgb-lib)); any other answer for an unknown TX (a 404 on the status,
  a 5xx, a timeout) fails the lookup, and no `Initiated` or `WaitingCounterparty` send can be
  failed for as long as that goes on.

### How it surfaces in rgb-lib

| Request | Sent by | The forwarder refuses | The forwarder fails (not listening, a non-2xx that is not JSON-RPC, a timeout) |
|---|---|---|---|
| `server.info` | `send_begin`, for each endpoint of each recipient | endpoint unusable; `ForwarderRefused` (the first) if no endpoint of the recipient is usable | endpoint unusable; `InvalidTransportEndpoints` ("no valid transport endpoints") if none is |
| `server.info` | `rust_only::check_proxy_url_via_forwarder` | `ForwarderRefused` | `Proxy` ("unable to connect to proxy") |
| `consignment.post` | `send_end`, the usable endpoints in turn | next endpoint; `ForwarderRefused` if none took the consignment | next endpoint; `NoValidTransportEndpoint` if none took it |
| `media.post` | `send_end`, after the consignment | `ForwarderRefused` | `Proxy` |
| `ack.get` | `refresh` of a send waiting for its ACK | the transfer's `failure`: `ForwarderRefused` | the transfer's `failure`: `Proxy` |
| `ack.post` (ACK, NACK) | `refresh` of a receive | the transfer's `failure`: `ForwarderRefused` | the transfer's `failure`: `Proxy` |
| `media.get` | `refresh`, receiving an asset the wallet does not know | the transfer's `failure`: `ForwarderRefused` | the transfer's `failure`: `Proxy` |
| `consignment.get` | `refresh` of a receive | "no consignment yet": no failure, the receive keeps waiting | the same |
| reject list `GET` | `refresh` and `provide_out_of_band_consignment` (receiving an IFA asset), `send_begin` (sending one) | `ForwarderRefused` | `RejectListService` (a non-200, a 200 without a length or chunked framing, or one holding text and no opout, included) |

- A refused endpoint next to a usable one is skipped like an unreachable one, so an invoice
  listing an unknown proxy beside a known one sends through the known one without an error.
- `consignment.get` is deliberately unchanged: upstream reads every failure there as "no
  consignment yet", and `fail_transfers` refreshes a transfer before failing it, so an error
  there would make an expired receive impossible to fail for as long as the forwarder refuses.
- **`fail_transfers`** refreshes a transfer first and gives up on its error, as upstream does.
  For a **send still in `WaitingCounterparty`**, `ForwarderRefused` and `InvalidForwardTarget`
  from that refresh count as "no change" (`a14cdbb`): otherwise a lapsed consent would keep it
  from ever failing. A receive keeps upstream's rule (a donation's witness may already be on
  chain), and so does an outage (the recipient may have answered).
- **No outgoing transfer is failed while the indexer knows its TX** (`fe7e1b0` for a refused
  ACK poll, `56e6186` for every path; with or without a forwarder). An `Initiated` or
  `WaitingCounterparty` transfer whose refresh changed nothing has normally broadcast nothing,
  but not always: a donation, inflation or burn broadcasts in its `*_end`, and a send in
  `try_complete_batch`, before the commit that records it, so a kill in between, or a backup (the
  app's seal) taken before and restored, leaves it `Initiated` (which a refresh does not touch)
  or `WaitingCounterparty` (whose ACK poll may answer "none yet", be refused, or bring a NACK)
  with its TX on chain. Before failing one, rgb-lib looks its TX up: unknown, it is failed as
  before; **in the mempool or in a block, `fail_transfers(Some(idx))` returns
  `CannotFailBatchTransfer`** and changes nothing; **if the lookup fails, it returns the
  `Indexer` error**. A NACK no longer fails a batch whose TX is known either.
  - It is not completed there. Upstream's completion path for those states is not
    `fail_transfers`: an `Initiated` transfer completes when the host calls its `*_end` again
    with the same signed PSBT (the broadcast repeats harmlessly; upstream's own tests do this
    after a simulated `send_end` crash), a `WaitingCounterparty` send when its ACKs come in. **So
    a host that gets `CannotFailBatchTransfer` for an `Initiated` transfer calls that `*_end`
    again** (`send_end` writes the signed PSBT into the transfer's directory as `signed.psbt`
    before it posts or broadcasts anything; for inflation and burn only the host has it). A
    transfer whose `*_end` ran after the backup it was restored from has neither: it stays
    `Initiated`, which is wrong but safe, where `Failed` was wrong and not. UTEXO's
    `v0.3.0-beta.43-bfa` answers its own case (prepare batches) the same way, with the same
    `CannotFail` outcome.
  - **The indexer's side of it**: an unknown TX must get `200 {"confirmed":false}` on
    `GET /tx/<txid>/status` and `404` on `GET /tx/<txid>/raw`, as electrs answers. Anything else
    (a 404 on the status, a 5xx, a timeout) fails every lookup, and no outgoing `Initiated` or
    `WaitingCounterparty` transfer can be failed for as long as it does: the forwarder and the
    backend relay those two endpoints as the indexer answers them.
- **Failing every expired transfer** runs each attempt in a savepoint (`1699a6a`). A transfer
  kept for one of the reasons above (a policy refusal of a receive, a TX the indexer knows, a
  lookup the indexer or the network fails, which the bulk call skips as UTEXO's beta.43 does) is
  skipped with none of its attempt's database writes kept (a receive's refresh marks the
  endpoint it got the consignment from as used, or stores the asset it receives, before the ACK
  is refused); files the attempt wrote (a downloaded consignment, media, the RGB stash) stay.
  The others are failed, and the call returns `true` only if a transfer was failed or refreshed.
  Failing that one transfer alone returns its error. A failing reject list (refused or not)
  blocks failing a receive the same way, as upstream does when the list is unreachable: that is
  the cost of failing closed.
- **An expired send whose last ACK comes in** is failed by `try_complete_batch` instead of
  broadcast, as upstream does, **unless its TX is on chain already** (`1051adc`, with or without
  a forwarder, for the same two reasons): it then goes on as one that has not expired (the
  broadcast is repeated, harmlessly) and ends in `WaitingConfirmations`. A lookup that fails
  fails that refresh, and the next one asks again. This is upstream's gap, still there on
  `v0.3.0-beta.43-bfa`, to report to UTEXO.
- `ForwarderRefused` is the forwarder's policy speaking: the same request gets the same
  answer until something changes on the forwarder's side (its allowlist, the user's consent).
  An outage keeps today's errors, which a retry can clear.
- `InvalidForwardTarget` fails `send_begin` at once, even when another endpoint of the
  recipient is usable: an invoice carrying such an endpoint is malformed or hostile, not a
  proxy that happens to be down. The endpoints of the wallet's own invoices are never checked
  when they are made, and a send made without a forwarder stores its recipient's endpoints
  unchecked; requests to such an endpoint are never sent, and the error surfaces as
  `InvalidForwardTarget` on `ack.get`, `ack.post` and `media.get`, and is swallowed where the
  table swallows failures (`consignment.get`: no consignment yet; `consignment.post`: next
  endpoint).

### What does not change

- Invoices, the transport endpoints stored with transfers and in the DB, and every value
  rgb-lib returns keep the real endpoint
  (`wallet::test::forwarder::proxy_traffic_goes_through_the_forwarder` checks the invoice and
  the stored transfer).
- With `forwarder_url` unset: the same client (upstream's `ProxyClient::new` /
  `RejectListClient::new`), the same requests, no extra header, the same errors, and upstream's
  `go_online` and `fail_transfers` behaviour, except that an expired send whose TX is on chain
  is not failed when its last ACK comes in (`1051adc`). Upstream reads a direct reject list's answer as the
  list whatever its status (a 503 page is an empty list); the fork leaves that path as it is (a
  test pins it). Worth reporting to UTEXO.
- Not routed by this patch, because the host already chooses those URLs: the indexer
  (`indexer_url`), VSS (its server URL), the multisig hub and DFNS (not used by the app).
  On the `-bfa` bases, `OnlineOptions::eth_rpc_url` (BFA validation reads an Ethereum
  RPC) is the same kind of host-chosen URL: point it at the forwarder too. The errors of those
  clients are theirs and have not been checked for URLs.
- **The Esplora and VSS clients follow redirects, and the Esplora one reads a proxy from the
  environment; rgb-lib cannot switch either off** (checked 2026-09-27, no change made):
  - esplora-client 0.12.3, used for the indexer (`bdk_esplora`) and the resolver (`rgb-ops`'s
    `esplora_blocking`), builds each minreq request inside its own methods. Its `Builder` offers
    a proxy, a timeout, headers and retries, nothing for redirects, and minreq's
    `with_follow_redirects` / `with_max_redirects` are set per request (default: follow, up to
    100). Its `blocking` feature turns on minreq's `proxy` feature, which reads `http_proxy`,
    `https_proxy` / `HTTPS_PROXY` and `all_proxy` / `ALL_PROXY` for every request that names no
    proxy, and rgb-lib names none (`NO_PROXY` is not read).
  - vss-client-ng (`ad63805`) builds each bitreq request inside `post_request`; bitreq follows
    301, 302, 303 (as a GET) and 307, up to a per-request `max_redirects` (100) that `VssClient`
    gives no way to set, and keeps the request's headers. bitreq's `proxy` feature is not on in
    this graph, so VSS reads no proxy from the environment.

  Turning either off needs a fork of esplora-client (or minreq) and of vss-client-ng. What holds
  instead: the forwarder and the backend never emit a 3xx on those routes
  ([The forwarder's side](#the-forwarders-side)), and **the host does not set `http_proxy`,
  `https_proxy`, `HTTPS_PROXY`, `all_proxy` or `ALL_PROXY` in its process** (a mobile app's
  process has none of them unless it sets them itself).

### Public API the host adapts to

- New: `OnlineOptions::forwarder_url: Option<String>` (`e7bdaa4`); `Wallet::go_offline()`
  (`ff6e73a`); `rust_only::check_proxy_url_via_forwarder(proxy_url, forwarder_url)` (`e7bdaa4`);
  `Error::InvalidForwarderUrl { details }` (`e7bdaa4`), `Error::ForwarderRefused { target,
  reason }` (`ccd23e9`), `Error::InvalidForwardTarget { details }` (`8a0255f`), all mirrored in
  the uniffi UDL, whose build now runs in `era.yml` (`87d5e88`).
- Changed behaviour, with `forwarder_url` set:

  | Call | Was | Now |
  |---|---|---|
  | `go_online` or `check_proxy_url_via_forwarder` with a `forwarder_url` without a path | accepted, its refusals unchecked | `InvalidForwarderUrl` |
  | `go_online` with a new `indexer_url` whose probe fails, a forwarder set before or by the call | error, still online on the previous indexer and forwarder | error, **offline**: old handles get `Offline` |
  | `go_online` with an invalid `forwarder_url` while a forwarder is set | error, still online on the previous route | error, **offline**: old handles get `Offline` |
  | `send_begin` | `InvalidTransportEndpoints` when every endpoint failed its probe | `ForwarderRefused` when a refusal is why; `InvalidForwardTarget` for an endpoint with userinfo or a fragment |
  | `send_end` | `NoValidTransportEndpoint` when no endpoint took the consignment; `Proxy` from `media.post` | `ForwarderRefused` when a refusal is why; `ForwarderRefused` from `media.post` |
  | `refresh` (a transfer's `failure`) | `Proxy` from `ack.get`, ACK, NACK, `media.get` | `ForwarderRefused`; `InvalidForwardTarget` for a stored endpoint with userinfo or a fragment |
  | reject list (`refresh`, `send_begin`, `provide_out_of_band_consignment`) | any answer read as the list | `ForwarderRefused`, or `RejectListService` for a non-200, a 200 without a length or chunked framing, or one holding text and no opout |
  | `fail_transfers(Some(send))`, the send's ACK poll refused | error, the send not failed | the send failed (`WaitingCounterparty` only) if the indexer does not know its TX; kept, with `CannotFailBatchTransfer` if it does, the `Indexer` error if the lookup fails |
  | `fail_transfers(Some(receive))`, a policy error in its refresh | error (`Proxy` for an outage) | `ForwarderRefused` / `InvalidForwardTarget` |
  | `fail_transfers(None)` with such a transfer among the expired | error, and nothing failed (rolled back) | that transfer skipped with none of its attempt's database writes kept, the others failed; `false` if no transfer was failed or refreshed |
  | `check_proxy_url_via_forwarder` | `Proxy` ("unable to connect to proxy") on a refusal | `ForwarderRefused`; `InvalidForwardTarget` |
  | NACK refused with a reason containing "Cannot change ACK" | the receive failed without its NACK | the transfer's `failure`: `ForwarderRefused` |
  | a 403 with `X-Era-Forward-Refused` and no valid session echo | read as the target's answer, silently | the same, and a warning in the wallet's log |

- Changed behaviour with or without a forwarder:

  | Call | Was | Now |
  |---|---|---|
  | `refresh` (and `provide_out_of_band_ack`) of an expired send whose last ACK comes in, its TX on chain | failed | goes on to the broadcast (`WaitingConfirmations`); a failed lookup fails that refresh (`1051adc`) |
  | `fail_transfers(Some(idx))`, an outgoing `Initiated` or `WaitingCounterparty` transfer whose refresh changes nothing | failed | failed if the indexer does not know its TX; **`CannotFailBatchTransfer`** if it does; the **`Indexer`** error if the lookup fails (`56e6186`) |
  | `fail_transfers(None)`, such transfers among the expired | failed | failed if unknown; skipped, and not counted as a change, if known or if the lookup fails (an `Indexer` or `Network` error of any transfer is now skipped, as on beta.43) |
  | `refresh` of a send whose ACK poll brings a NACK, its TX on chain | failed | left as it is; a failed lookup fails that refresh (`56e6186`) |
  | `fail_transfers` (`skip_sync` too) of an outgoing `Initiated` or `WaitingCounterparty` transfer | no indexer request | one lookup of its TX before it is failed: failing one needs the indexer |

### Where it is

`src/api/forwarder.rs` (validation of `forwarder_url`, the client, the target check and the
headers in `Forwarder::request`, `Forwarder::refusal` and `refusal_reason`, `Forwarder::scrub`,
the client-level tests), `ProxyClient::post` / `ProxyClient::call` and
`RejectListClient::get_forwarded` (the one place each builds, sends and reads a routed request),
`has_checked_end` next to it in `src/api/reject_list.rs`, `WalletOnline::forwarder` /
`proxy_client` / `reject_list_client` / `check_proxy_endpoint`, `go_online_impl`,
`go_offline_impl`, `batch_tx_known`, `try_fail_batch_transfer`, `fail_transfers_impl`,
`try_complete_batch` and `refuse_consignment` in `src/wallet/online.rs`,
`TryFailBatchTransferOutcome::CannotFail` in `src/wallet/objects.rs`, `DbTxn::savepoint` in
`src/database/mod.rs`, `Wallet::go_offline`, `utils::check_proxy_routed`,
`OnlineOptions::forwarder_url`, `OnlineData::forwarder`, the three error variants,
`rust_only::check_proxy_url_via_forwarder`.

### Guard and tests

**The proxy forwarder guard** (`era.yml`, "Proxy and reject-list clients only through the
wallet's route"): wallet code reaches an RGB proxy or a reject list only through
`WalletOnline::proxy_client`, `reject_list_client` and `check_proxy_endpoint`, which read the
forwarder `go_online` stored and take no route from their caller. The step lists every mention
in code of the type names `ProxyClient` and `RejectListClient` (a construction, a function value,
a type in a signature, an import: so a type alias, a `use … as` or a macro naming them too;
`a140543`) and of `check_proxy`, `check_proxy_routed`, `check_proxy_url` and
`check_proxy_url_via_forwarder` (called, taken as a value or imported), comment lines left out,
in `src/` outside the three client files and the tests (`8692b69` and `a140543`; before, only
`::new(`, `::new_routed(` and the checks followed by `(`), and fails unless the list is exactly
the expected set: the three helpers (signatures and bodies), upstream's `check_proxy`
(definition, body, test, and `lib.rs`'s import of it), `rust_only::check_proxy_url` (no wallet,
so no forwarder) and `check_proxy_url_via_forwarder` (given one explicitly) with their
definitions, the crate's imports of the two types, one parameter of type `&ProxyClient`, and the
TLS tests in `src/api/mod.rs`. A call site passing `None` to a routed constructor, a direct
constructor or check, a function value of either, a qualified path, a type alias, an aliased
import, a macro building a client, or a second copy of a helper's line fails it (checked by
injecting each). It is still a grep: an identifier that is not the type's name escapes it
(a generic over a trait the clients implement, say), where none exists today.

**Why not the compiler.** Making `ProxyClient::new` `pub(super)` was asked for and not done:
wallet code must reach a direct client for the case where no forwarder is set, and upstream's
`check_proxy` (`utils.rs`) and `rust_only::check_proxy_url` need one without a wallet at all.
Rust's visibility cannot tell the three helpers in `online.rs` from other code in the same
module, and narrowing `new` alone would leave `new_routed(url, None)` building the same direct
client from anywhere in the crate, so it would move the hole, not close it. Closing it would take
moving the route decision and upstream's `check_proxy` into `api`, which reshapes upstream code
every carry would then have to rebase.

`cargo test --locked --lib --features esplora,vss -- api::forwarder:: wallet::test::forwarder::`
(57 tests and a child test, local mockito servers and raw sockets, no regtest). Every forwarder
in the tests has a path, and every request must arrive on it. The tests of both modules share one
`serial_test` key and run one at a time: each holds several mockito servers, whose pool is 20 on
macOS, and run side by side they deadlocked waiting for one more (`29a49bb`).

- Client level (`api::forwarder::tests`, 22 and the child): every proxy method arrives with the
  right target and kind and nothing reaches the proxy, the reject list the same,
  `check_proxy_url_via_forwarder`; the target keeps scheme, port, path and query, and is
  `url::Url`'s serialization; userinfo or a fragment refused on every method before any
  request; a refusal is `ForwarderRefused` on every method, the check and the reject list,
  while a 403 without the header, the header on another status, or a missing or different
  session echo is not, and is logged without the path; the reason is cut by characters; a
  forwarded reject list fails closed on 403, 503, 404, 500, 307, 203, 204 and 206, on a 200 that
  holds text and no opout, and on a 200 whose end the client cannot check (answered from a raw
  socket: ended by the close, whole or cut, empty or not, chunked followed by another coding; a
  length longer than the body; chunked without its last chunk), while a framed one, empty or
  not, is read and a direct one is read as upstream reads it; request errors name the target, not the forwarder;
  the forwarder's client ignores `HTTP_PROXY` (in a child process started with it set, after
  showing that an ordinary client does go there); without a forwarder the request goes direct
  with no `X-Era-*` header; a forwarder that is down, answers 403 or redirects: an error, and
  the proxy is never contacted; URL validation, a URL without a path refused; the `Debug` form
  without the path.
- Wallet level (`wallet::test::forwarder`, 35), each request sent from the code that sends it
  in production and required at the forwarder with both headers, never at the real host:
  `server.info` through `send_begin`; `consignment.get`, a NACK and `ack.get` through
  `refresh`; `consignment.post` and `media.post` through `post_transfer_data` (`send_end`
  needs a PSBT that spends real allocations); the ACK through `ack_consignment`, `media.get`
  through `fetch_and_save_attachments` and the reject list through `get_reject_list` (all three
  follow a consignment validated against a chain). Then: the reject list fails closed (and an
  empty 200 is an empty list); the refusal in `send_begin` (alone, the first of two, next to a
  usable endpoint), in `send_end` (alone, and before a usable endpoint that then takes the
  post), in the ACK poll, and in `consignment.get`, where it leaves the receive waiting and
  failable; a refusal reason never read as the proxy's "Cannot change ACK", and the proxy's own
  "Cannot change ACK" answer failing the receive with a warning in the wallet's log; a refusal
  without the session echo in the wallet's log, without the path; a refused or unroutable send
  failed by `fail_transfers` once the indexer has said it does not know the TX, and kept, alone
  and in the bulk call, when the TX is in the mempool, in a block or cannot be looked up; the
  same for an `Initiated` send and for a `WaitingCounterparty` one whose ACK poll answers "none
  yet" (`CannotFailBatchTransfer` or the `Indexer` error alone, skipped without a change in the
  bulk call); a NACK failing a send only when its TX is unknown; a receive failed as upstream
  does, without a lookup, whatever TX its batch carries; an expired send whose last ACK comes in
  failed only if its TX is not on chain, going on to the broadcast if it is, and the refresh
  failing if the lookup does; an outage still keeping a send from failing, a blocked receive
  skipped by the bulk call with none of its attempt's database writes kept, a bulk call that
  only skips returning `false` and marking no backup as needed; a forwarder that
  is down keeps today's errors in `send_begin` and `send_end`; userinfo or a fragment in an
  invoice endpoint fails `send_begin`, next to a usable one too; `go_online` switching the
  forwarder on and off, refusing bad URLs without changing a direct route and going offline
  from a forwarded one, going offline when the probe of a new indexer fails (with the forwarder
  kept or dropped by the call; indexer and proxy traffic both checked) and not without a
  forwarder; `go_offline`; the invoice and the stored transfer keep the real proxy; without a
  forwarder the proxy is contacted directly.

Mutations, 2026-09-27. First review: any one call site or helper going direct or passing `None`
(11), the refusal ignored by the proxy client, the reject-list client, `check_proxy`,
`send_begin` or `send_end`, a refusal reported next to a usable endpoint, a 403 taken as a
refusal without the header, the header taken on another status, the reason not cut (9),
userinfo, a lone password or a fragment let through, `check_proxy` or `send_begin` swallowing
an invalid target (5), a forwarded non-2xx reject list read as a list, the forwarder's URL left
in a send error, and those of `e7bdaa4` (redirects followed, any host accepted, a wrong target
header, no kind header, the forwarder cleared before validation): each fails at least one test.
Second review: its 20 mutations (four had survived: `send_end` returning the first refusal,
`send_begin` keeping the last one, the system proxy honoured, the reason cut by bytes) and the
new ones (a failed probe keeping the stale state or ignoring the previous forwarder, going
offline without a forwarder, `go_offline` doing nothing; the session echo not required,
compared by prefix or required at the root; any error text taken for "Cannot change ACK"; a
refused send not failable, a receive failable, `InvalidForwardTarget` not counted, the bulk call
aborting on a policy error or skipping every error; any 2xx or a 200 without an opout read as a
list, an empty 200 refused, allow lines not counted): each fails at least one test. Third
review, 23 mutations, each failing at least one test: the chain lookup skipped, its error read
as "unknown", a mempool TX read as unknown, every TX read as known; the same check in
`try_complete_batch` dropped or its error read either way; no savepoint, a savepoint committed on
a skip, the change counted before the attempt; the NACK's warning arm dropped, any error text
matched again; a refused URL keeping a forwarded route, or dropping a direct one; a URL without
a path accepted, the unechoed refusal not logged, the echo not required, the wallet's logger not
handed over; the framing check dropped, a length or chunked framing not honoured, chunked
anywhere in the codings taken, every version taken as HTTP/2. Fourth review, 11 mutations,
each failing a test: the new chain check dropped, its lookup error read as "unknown",
`Initiated` or `WaitingCounterparty` left out of it, receives let in, a kept transfer counted as
a change by the bulk call or silent in the single one, an `Indexer` error aborting the bulk call,
the NACK's check dropped or its lookup error read as "unknown". Scrubbing the URL from body and
decoding errors has no effect to test: reqwest 0.13 puts no URL there.

## 5. Backup restore checks

`45c3b39` and `777fe57`, then, after a review of those on 2026-09-27 ("pass with issues": every
server-made backup it tried was refused by `restore_from_vss_expecting` with encryption on, and
the rest was open), `dcc9654`, `55cb0c9`, `b8df12d`, `6921d07`, `bb85811` and `d6357cf`, and after
a review of those (which restored every genuine backup it could make at `d82e21a`: whole-block
files, a 6.7 MB one, chunked VSS backups including exactly 2 MiB and 2 MiB + 1), `4ea8ccd`,
`c188e63`, `ba96a80` and `3988f38`; then `b761c25`, after a review of `era_rgb` found the
restored wallet placed before any of it was on disk. They cover the VSS restore and, for the
decryption, the file backup too (`Wallet::backup` / `restore_backup`, the app's D6 seals).

### Why

`restore_from_vss` trusted three answers of the VSS server, each read at its own moment: the
manifest was read twice (once to decide the rename of a plaintext backup's `wallet/` directory,
once more inside `download_backup` to decide the decryption), the name the server gives the
wallet (`backup/fingerprint`) became a directory name as it came, and a backup the manifest
marks as unencrypted was restored with nothing to authenticate it. The app's bridge (`era_rgb`)
reads the fingerprint and the manifest itself before calling rgb-lib and checks the result
after, but a server answering the bridge's reads and rgb-lib's differently gets past both: a
"plaintext" backup of its own making, renamed to `../<anything>`, lands outside the data
directory before the bridge's post-check runs (found in the review of `era_rgb`, task T1.1b).
It also wrote the server URL and the store ID into the restore log it leaves in the target
directory.

The review of `777fe57` found `restore_from_vss` still open, and it is what the bridge calls at
the pinned rev: it took a server-made plaintext backup with encryption enabled and extracted
entries anywhere inside the target directory (over another wallet's database). Both restores
extracted entries outside the wallet directory. Both
decryptors, VSS and file, stopped cleanly when their input ended on a block boundary, so a
backup cut short restored a correct prefix and an empty one restored nothing. And a server's
metadata or manifest could panic the restore, or abort the process on an allocation.

### What the bridge calls

**`restore_from_vss_expecting(config, target_dir, expected_fingerprint)`**, with encryption
enabled in `config`. `restore_from_vss` is deprecated in the fork (`d6357cf`; it keeps its
signature and still builds the bindings): it refuses everything the expecting variant refuses
except a genuine backup of another wallet, which it cannot tell. With the expecting variant, the
bridge's own reads before the call, its check of the directory after it and its
manifest-file heuristic are redundant; emptying the data directory after a failure is harmless.
The restored directory keeps the name the backup carries, fingerprints being compared ignoring
case: the bridge, which always lowercases, gets its canonical name back.

### What a restore checks

Both restores, each answer of the server read once, in this order:

1. (expecting only) `expected_fingerprint` is 8 hex characters, else
   `Error::InvalidFingerprint`, before anything is requested or written.
2. The manifest. With encryption enabled in the config (the default), a backup it marks as
   unencrypted is **`Error::VssBackupUnencrypted`** (new variant, in the UDL), before any of it
   is downloaded: decrypting is what authenticates a backup (`777fe57` for the expecting
   variant, `6921d07` for both). To restore a plaintext backup, pass a config with encryption
   disabled, as upstream's own plaintext tests do.
3. The wallet the server names (`backup/fingerprint`): the expected one, else
   `Error::FingerprintMismatch`; without an expected one, 8 hex characters, else
   `Error::VssError`. Fingerprints are compared ignoring case (`4ea8ccd`): rgb-lib names a wallet
   directory after the master fingerprint exactly as the host gives it (`setup_new_wallet`), so a
   wallet created with `928E8C83` lives in `928E8C83/` and its backups say so; `b8df12d` refused
   upper case, and with it the genuine backups of such wallets, which `d82e21a` restored.
4. The download (`55cb0c9`, `ba96a80`), all `Error::VssError`: the manifest must describe
   between 1 byte and `MAX_VSS_BACKUP_SIZE` (256 MiB; a wallet backup is a few megabytes) in no
   more chunks than 1 MiB chunks take (so at most 256 requests), checked before anything is
   downloaded. Every upload has split its data into chunks of `VSS_CHUNK_SIZE`, 4 MiB until
   `88adde0` (March 2026) and 1 MiB since, so the backups of both fit; `MIN_VSS_CHUNK_SIZE` holds
   the bound, with a compile-time check against `VSS_CHUNK_SIZE`. No buffer is sized by the
   manifest's numbers; the data must add up to its `total_size` exactly (a single backup's data,
   or the chunks, none empty, the download stopping at the one that goes past it), which every
   upload has written. An encrypted backup's metadata, a 32-byte salt and a 19-byte nonce in hex,
   is read first, and each piece is decrypted as it arrives: a piece that does not decrypt ends
   the download there, instead of after the whole backup is in memory. A short nonce is an
   error, not a panic, in `encrypt_data` and `decrypt_data` too, and in a file backup's public
   data.
5. The decryption of an encrypted backup ([below](#what-decryption-proves)).
6. An encrypted backup names its wallet inside (its first entry), where the server cannot change
   it: that must be the wallet the server named (ignoring case), else
   `Error::FingerprintMismatch` (`777fe57` for the expecting variant, `b8df12d` for both).
   `<target_dir>/<name>` must not exist (`Error::WalletDirAlreadyExists`), the name being the
   one the backup carries: inside it for an encrypted backup, the server's for a plaintext one.
7. The extraction (`b8df12d`), of the wallet directory only: for an encrypted backup the one it
   names, for a plaintext one its sanitized `wallet/`, mapped straight onto
   `<target_dir>/<name>` (no rename). Entry names are resolved by their components (`/`
   and `\`; `..` within the archive only; no absolute name, NUL or `:`), and an entry outside the
   wallet directory is skipped, counted in the log. It goes into a staging directory next to the
   wallet's (`.vss_restore_<fingerprint>_<nanos>`), renamed into place once complete; a backup
   holding no file of the wallet is `Error::VssError`. Symlink entries are written as files, as
   upstream wrote them.
8. A restore that fails leaves nothing behind that it made, and nothing it did not make is
   touched (`bb85811`, `c188e63`): not the staging directory, not its log, and not the target
   directory, or a parent of it, if it made them. The directories are created one component at a
   time and only those `fs::create_dir` made are removed (a target reached as
   `missing/../there` leaves `there` alone); the log is a file of its own, created with
   `create_new` as `vss_restore_<unix time>`, with a `_<n>` suffix when that name is taken, so a
   file that was there is neither written to nor removed. A completed restore keeps its log in
   the target, as upstream; the log names neither the server URL nor the store ID (`45c3b39`).
9. **The wallet is placed durably** (`b761c25`). A rename can reach the disk before the data it
   moves (ext4's delayed allocation and jbd2 commits, f2fs checkpoints), so a power loss could
   leave a restored wallet in place with short files: a short `rgb_lib_db`, short consignments,
   a short `bdk_db_watch_only`, which rgb-lib self-heals and the app would then seal and upload
   over the good copy on the server. Before the rename, every file and directory under the
   staging directory is fsynced (each directory after what it holds); after it, the directory
   that holds the wallet, so the rename is on disk when the restore returns. Every sync's error
   fails the restore: before the rename the staging directory goes, after it the wallet
   directory goes, so nothing is left in place either way. Directories are synced where they can
   be opened as files (unix: Android and iOS included). The host need not sync the tree again.

### What decryption proves

Both backup kinds use the same STREAM construction: XChaCha20-Poly1305 over 239-byte blocks,
each sealed under a nonce made of the backup's random 19-byte prefix, the block's position (u32,
big-endian) and a last-block flag. The key comes from the signing key (VSS: HKDF-SHA256 with the
metadata's salt) or the password (file: scrypt with the backup's salt).

- **Origin: yes.** Only the key's holder can seal a block: a block the server made or changed
  fails (`VssError` "decryption failed…", `WrongPassword` for a file).
- **Order: yes.** A block at another position fails.
- **Completeness: yes, since `dcc9654`.** The final block is always short (a tag alone when the
  data fills its last block, and for no data at all), so input that ends on a block boundary,
  empty input included, is refused as truncated (`VssError` "…truncated", `IO` "…truncated"
  for a file, whose password is right if every block so far authenticated). Bytes after the
  final block shift the block boundaries, and fail.
- **Freshness: no.** An older genuine backup, served with its own manifest and metadata,
  decrypts and restores as well: a server can roll a wallet back to any backup it kept. rgb-lib
  cannot tell; a host that needs to has to compare versions itself.
- **Whose wallet: only with `restore_from_vss_expecting`**, which checks the name the archive
  carries inside, under the same authentication.

**What a damaged file backup gives.** A seal is a zip holding `backup.enc` (the stream) and
`backup.pub_data`. Cut anywhere, or otherwise not a zip, it is `Error::InvalidFilePath`, the
variant a missing file gets: the outer zip breaks first. Only a stream cut on a block boundary
inside a valid outer zip gives `IO` "…truncated". A stream cut mid-block, or with a block changed,
gives **`WrongPassword`, exactly as a wrong password does**. So `WrongPassword` does not prove the
password (the seal key) is wrong: it means wrong key *or* damaged seal, and a bridge must not act
on it as if the key were wrong (discarding or rotating it, telling the user so).

**Backward compatibility.** The encryptors always wrote the short final block, so nothing they
wrote is refused. `src/wallet/test/fixtures/d82e21a/` holds what `d82e21a` (the rev the app
pinned before these changes) wrote with its own code: stream vectors (lengths 0, 1, 238, 239,
240, 477, 478, 479 and 1000, under a fixed key and metadata), the four values a VSS server holds
for an encrypted backup of a new watch-only wallet, and a `Wallet::backup` file of that wallet
(default scrypt parameters, like a seal), with listings of what that rev's own restore made of
them. `wallet::test::era_fixtures::generate_backup_fixtures` wrote them, run on a checkout of
`d82e21a` (its doc has the command). The tests decrypt every vector (and today's `encrypt_data`
writes the same bytes), restore the VSS backup with both restores and the file backup with
`restore_backup`, and compare the restored tree file by file with the listings. `encrypt_file`
and `decrypt_file` now fill each block before judging its size (`File::read` may return less
than asked); for the files rgb-lib writes that changes nothing.

### Limits

- A single VSS response is capped at 1 GiB inside vss-client-ng (`MAX_RESPONSE_BODY_SIZE`),
  which rgb-lib cannot lower without forking that crate: a server can still make one response
  that large before the size checks see it.
- With encryption disabled in the config, the server's plaintext is restored as it comes, now
  limited to its `wallet/` directory, and its entries are not size-capped: a zip bomb fills the
  disk. The app does not disable encryption.
- `delete_backup` still trusts the manifest's `chunk_count` (upstream's; not a restore).
- `restore_backup` (file) still leaves its `restore_<ts>` log, and the target directory, after a
  failure, as upstream does (the bridge removes the log); step 8 is VSS's. It stages and renames
  nothing either: it extracts the wallet in place and syncs nothing, so step 9 has nothing to
  order there, and a host that needs the restored files on disk syncs the tree after it (the
  bridge does).
- **An interrupted chunked upload leaves no restorable backup.** `upload_chunked` writes each
  chunk under the previous backup's keys (`backup/chunk/<i>`) and the manifest and metadata
  last, so an upload that stops after its first chunk leaves the previous manifest describing
  chunks that are partly the new upload's: the previous backup is gone, and the restore fails at
  the first foreign chunk (its decryption), until an upload completes. upstream's comment said the
  previous backup survives; `3988f38` corrects it. The layout is unchanged here (the app's task
  T1.5).

### Tests

`cargo test --locked --lib --features esplora,vss -- wallet::vss::tests:: wallet::backup::tests::`
(45 and 3 tests, offline; the first in `era.yml` since `45c3b39`, the second since `dcc9654`). A
scripted VSS server answers `getObject` per key and per read, and counts the reads, so the host's
read can be told the truth and rgb-lib's something else.

- `777fe57`: another wallet (`deadbeef`, `../escape`) on the second fingerprint read, a
  plaintext backup on the second manifest read, both refused with nothing of the backup fetched
  or written; an encrypted backup of another wallet, downloaded and decrypted, then refused
  before anything is written; an expected fingerprint that is not one refused before any
  request; the expected wallet restored with one read of each key; the manifest read once.
- The third review: every backup the server can make refused by both restores with encryption
  on (its B1 probe as a test: marked plaintext, marked encrypted but plaintext, no manifest or
  one that is not JSON, another key, no metadata, chunks of two backups, ciphertext and
  plaintext chunks either way round, no chunk, plaintext chunks), and a plaintext one taken with
  encryption off; entries outside the wallet directory (above it, absolute, another wallet's,
  `wallet/`, climbing out with `..`, a drive name) never written and another wallet's database
  left as it was, encrypted and plaintext, a backslash-separated entry extracted; upper-case,
  empty, slashed and absolute server names refused; an encrypted backup of another wallet
  refused by `restore_from_vss`; no wallet file, or a damaged entry, leaving neither a wallet nor
  a staging directory; malformed metadata, impossible manifests (nothing downloaded), data not
  adding up to its manifest (the third chunk never read), and a correctly chunked backup
  restored; a failed restore leaving nothing, into a missing nested target or next to another
  wallet; a completed restore's log without the URL or store ID; every cut of a VSS and of a file
  ciphertext refused, as truncated at a block boundary; a file backup with a short nonce an
  error; the `d82e21a` fixtures.
- The fourth review: a backup named `928E8C83` restored under that name with no, lower, upper and
  mixed expected fingerprints, the server naming it in either case, encrypted and plaintext, and
  another wallet refused whatever the case; a real wallet created with an upper-case master
  fingerprint, uploaded through `VssBackupClient` to a mock server that keeps what it is given,
  restored file for file by both restores; a restore refused, and the directory untouched, when
  the wallet's directory exists; a failed restore removing only what it made
  (`missing/../there`, a target blocked by a file, files carrying the log's names); manifests
  naming more chunks than 1 MiB chunks make refused before any download; a backup of more than
  4 MiB restored in 4 MiB and in 1 MiB chunks; the download stopping at the first chunk that
  does not decrypt (reads 1-0-0, 1-1-0, 1-1-1).
- `b761c25`, through a test-only record of syncs and renames and a hook that fails a chosen sync:
  every file and directory of the restored wallet synced under its staging name before the
  rename, each directory after its contents, the target directory right after the rename and
  nothing else; a failing file or directory sync leaving nothing and no rename, a failing sync
  after the rename leaving nothing either. The record stands for the syncs: the test cannot see
  the disk's own ordering.

Mutations: `777fe57`'s 7 and the two log lines put back (round 2); round 3, 28 (truncation 4,
metadata and sizes 9, extraction 9, plaintext 2, cleanup 4), each failing a test. A tenth size
mutation, no longer refusing zero chunks, survived: the size check already refuses them, and the
clause went. Round 4, 15, each failing a test: upper case refused again, either comparison made
exact, the directory named after the server for an encrypted backup, the existence check
dropped; either cleanup dropped, an existing log file reused, a directory that was there counted
as made; the chunk bound per byte again or per 4 MiB chunk, decryption at the end of the
download, the truncation check dropped, a whole block held back. `b761c25`, 7, each failing a
test: the file syncs or the directory syncs dropped, parents synced first, the rename first,
the parent sync dropped, sync errors ignored, the wallet kept after a failed parent sync.

## 6. Completing an own unrecorded spend (CC-99)

`5cab571`, `f82b1c1`, `27d2a5a`, `c1feef6`, `4093572` and `106a00c`, after the design of
2026-09-28 and its adversarial review (the deviations are [listed](#deviations-from-the-design)),
then `74664d9` to `690f041` after a review of those commits (no blocker, no major).
It has to be in place before the app's lifecycle service (T1.3), which pauses the wallet between
any two operations.

### Why

A colored spend whose transaction reached the network while the wallet's database did not
record it leaves the transaction's inputs `exists && !spent` in the database and spent in BDK.
Upstream's singlesig consistency check (`singlesig.rs`, `wallet_specific_consistency_checks`)
then refuses every `go_online` with `Inconsistency` (`RestoredBackupInconsistent` after a VSS
restore). Its only repair, the orphan reconcile in the public `sync`, needs an `Online` handle
that `go_online` never returns in that state, and would mark the inputs spent without the RGB
transition or the transfer's status, dropping the change from the stash for good.

Three sequences lead there:

- **S1**: the broadcast reaches the network, its answer is lost (a forwarder answering 502 after
  relaying, a dropped connection) and so is the lookup `broadcast_tx` makes after a failed
  `POST /tx`; the operation rolls back. The app's pause seals that state.
- **S2**: the process dies between the broadcast and the commit.
- **S3**: the wallet is restored from a seal or VSS backup taken after the operation's `*_begin`
  and before its commit.

Three operations can do it: a donation's `send_end`, the `refresh` that broadcasts an ACKed send
(`try_complete_batch`), and `drain_to_end`. `create_utxos` and `send_btc` spend vanilla coins
only.

### What the host sets and reads

| API | Meaning |
|---|---|
| `OnlineOptions::complete_unrecorded_spends: bool` | `#[serde(default)]` false, UDL `= false`. False: upstream's check, unchanged (the same requests, writes and errors). True: the check completes own unrecorded spends it can prove (below). Ignored with `skip_consistency_check`, and by multisig and MPC wallets (their checks are no-ops). |
| `Wallet::completed_spends() -> Vec<CompletedSpend>` | What the last `go_online` completed. Empty when it completed nothing, skipped the check or failed, and while offline (`go_offline` drops it). Each `go_online` clears it first, before anything that can fail (`74664d9`). Rust API only (not in the UDL). |
| `CompletedSpend { txid, batch_transfer_idx, previous_status, donation, confirmations }` | `batch_transfer_idx` and `previous_status` are `None` for a drain; the transfer is now `WaitingConfirmations`. `confirmations` is the indexer's answer, 0 for its mempool. |
| `Error::UnrecordedSpend { txid, reason, batch_transfer_idx }` | New. The wallet recorded the spending TX and the indexer knows it, but its record or files do not allow completing it. Deterministic: retrying does not help. `reason` is a code of `UnrecordedSpendReason` (below). Nothing of this spend is written and the database is unchanged; with `stash-refused`, spends of the same `go_online` consumed before it stay in the stash (S2's state), which the next `go_online` skips (the rustdoc says so since `df04de0`). |
| `Error::UnrecordedSpendUnseen { txid, batch_transfer_idx }` | New. BDK has the coins spent by a TX this wallet recorded, and the indexer answers that it does not know it: lag, or a TX the wallet applied locally that left every mempool. Retryable. |
| `Error::Inconsistency` / `RestoredBackupInconsistent` | Unchanged variants. With the option on, the spent-coin refusal means "spent by a TX this wallet did not record" and its `details` is upstream's text followed by `; spenders: [..]; reason=<code>`; `Error::inconsistency_reason() -> Option<InconsistencyReason>` reads the code. `None` for every other error and without the option. |
| an indexer error | The lookup failed: retryable, nothing written. |
| `Wallet::vanilla_tx_record(txid) -> Option<VanillaTxRecord { txid, type, pending }>` | New (`4093572`). The record a `*_begin` with `dry_run = false` (or a `*_end`) left of a vanilla TX; `pending` while it holds its reservations. What the bridge's drain guard needs (below). |

`InconsistencyReason`: `spender-not-recorded` (another install of the wallet, a copy older than
the operation, a drain begun with `dry_run = true`), `no-canonical-spender` (the TX that created
the coin was replaced or reorganized away: not CC-99), `later-spend-unrecorded` (an output of an
own spend was spent by a TX this wallet has no record of).

`UnrecordedSpendReason`: `transfer-failed`, `unexpected-status`, `record-ambiguous`,
`record-kind-unsupported`, `transfer-data-missing`, `record-mismatch`, `stash-refused`, and
`external-operation`, reserved for the `-bfa` base (never produced here; see
[carrying](#carrying-it)).

The TXID stays in the fork's errors, as in `MinFeeNotMet` / `FailedBroadcast` and every
broadcast line of the wallet log (design question 4): it is the only name of a drain. The bridge
drops it from what it hands the app's log (below).

### When it completes

`D` is upstream's divergence: database TXOs `exists && !spent` minus BDK's `list_unspent()`,
after the check's two full scans in this very `go_online`. The completion
(`src/wallet/unrecorded_spends.rs`, `plan` then `apply`) runs only when all of this holds, for
every coin of `D`:

| # | Claim | Proven by |
|---|---|---|
| P1 | T spends the coin | the unique canonical TX (`bdk_wallet.transactions()`) with an input on it |
| P2 | T is this wallet's | exactly one record names T's TXID: one outgoing batch transfer (incoming send-to-self rows ignored) or one `wallet_transaction` of type Drain. The TXID commits to every input and output. |
| P3 | T reached the network | `get_tx_confirmations(T)` is `Some`: the bar `broadcast_tx` uses to count a broadcast |
| P4 | nothing after T was lost | no colored output of T is spent in BDK unless its database row is spent |
| P5 | the record is T's | a send: status `Initiated` or `WaitingCounterparty`; `transfer_data.txt` and `fascia` read as `get_transfer_end_data` reads them; every main transition a Transfer; the fascia's witness is T; the fascia's contracts are exactly the batch transfer's assets (user-driven and moved along: a fascia carries a bundle for each, pinned by `plan_counts_the_assets_moved_along`); the coins of the batch's Input colorings are inputs of T; `signed.psbt`, if there, is T's. A drain: still pending, its reservations exactly T's inputs. |
| P6 | coverage | every coin is attributed to such a T; one that is not and nothing is completed |
| P7 | nothing left | the divergence recomputed after the writes is empty, else `Error::Internal` |

The ACKs, the expiry and a NACK play no part: the chain decides, as in `56e6186` and `1051adc`.
No RGB proxy is contacted and nothing is broadcast.

Precedence, so the verdict does not depend on order: an unexplained coin (Inconsistency) before
a failed lookup, a failed lookup before an unknown TX (`UnrecordedSpendUnseen`), an unknown TX
before a record that cannot be completed (`UnrecordedSpend`), which therefore always means the
indexer knows T.

### What is completed, and what is refused

| Wallet state (after the check's scan) | Origin | Result |
|---|---|---|
| outgoing batch `Initiated`, donation, `signed.psbt` there | S1/S2 of `send_end`; its recipient broadcast it from the consignment | completed; stash witness: the TX this wallet signed, as `send_end` consumes it |
| outgoing batch `Initiated`, donation, no `signed.psbt` | S3: copy after `send_begin` | completed; stash gets the fascia as stored, with the TX unsigned |
| outgoing batch `WaitingCounterparty`, ACKs in any state, expired or not | S1/S2 of the `refresh` that broadcast it (its failure recorded, the ACK committed); S3 after `send_end` | completed; fascia as stored, as `try_complete_batch` consumes it |
| outgoing batch `Initiated`, not a donation, no `signed.psbt` | S3: copy after `send_begin`, the send finished elsewhere through `refresh` | completed, as above |
| pending Drain whose reservations are T's inputs | S1/S2 of `drain_to_end`; S3 after `drain_to_begin(dry_run = false)` | completed; reservations released |
| several of these at once | one `refresh` that broadcast two sends, a send and a drain | all completed, one commit; one refused and none is committed (a stash refusal keeps the spends consumed before it in the stash, below) |
| any of these after a VSS restore | S3 via VSS | completed; `.vss_restored` removed after the commit |
| a spender no record of this wallet names | another install (CC-27), a copy older than the operation, a `dry_run = true` drain, a transfer deleted then broadcast | `Inconsistency` `spender-not-recorded` (`RestoredBackupInconsistent` after a VSS restore) |
| a coin without a canonical spender | its creating TX replaced or reorganized away | `Inconsistency` `no-canonical-spender` (not CC-99; design question 6) |
| an output of T spent by an unrecorded TX | a chain of lost transfers from an old copy | `Inconsistency` `later-spend-unrecorded` |
| the lookup fails | indexer or forwarder down | the indexer's error |
| own record, BDK has T, the indexer does not know it | indexer lag; T applied locally and evicted | `UnrecordedSpendUnseen` |
| own batch `Failed` | failed while T was unknown, then broadcast (a donation's recipient, a late relay) | `UnrecordedSpend` `transfer-failed`: terminal in v1, by the lead's decision (a donation cannot be cancelled after a first `send_end` attempt) |
| own batch `WaitingConfirmations` / `Settled` with inputs unspent; a drain no longer pending | unreachable: committed with the spent marks | `unexpected-status` |
| two outgoing batches, or a batch and a vanilla TX, with T's TXID | | `record-ambiguous` |
| inflation, burn or link; a `CreateUtxos` / `SendBtc` record | not exposed by the bridge; unreachable | `record-kind-unsupported` |
| `transfers/T/`, its info or fascia missing or unparsable, `signed.psbt` unparsable | `delete_transfers`, damage | `transfer-data-missing` |
| fascia of another TX or other assets, input colorings outside T, reservations other than T's inputs, `signed.psbt` of another TX | damage, a misplaced file | `record-mismatch` |
| the stash refuses the transitions | a fascia rgb-ops will not consume | `stash-refused`: nothing of the refused spend is kept, nothing panics; spends of the same plan consumed before it stay in the stash, since rgb-ops commits each fascia on its own (S2's state, skipped by the next `go_online`) |
| the disk refuses a store of the stash | a full disk, a read-only directory | `Error::IO`, retryable (a panic inside rgb-ops before `ac6724d`) |
| option off, or `skip_consistency_check` | | upstream: no lookup made |

Outside it on purpose, because none is a divergence: a broadcast that never reached the network
(nothing is spent anywhere; `go_online` passes and the in-session retries or `fail_transfers`
finish it), a TX the indexer does not show yet in S1 (`go_online` passes; a later one completes
it), and the vanilla `create_utxos` / `send_btc`.

### The order of the writes

1. **Divergence and attribution** (BDK and the database, no writes): P1, P2, P4.
2. **Chain**: one `GET /tx/<txid>/status` per own spend (plus `/blocks/tip/height` or
   `/tx/<txid>/raw`), in TXID order: P3. No second scan.
3. **Records and files**: P5, and the donation's witness.
4. **Stash**: the check's runtime, taken before the scans as upstream takes it, stops persisting
   on drop (`require_explicit_persistence`), so a consume that fails half way stores nothing and
   cannot panic in `Drop`; bundles the stash and the index already hold are left out
   (`RgbRuntime::fascia_unknown_part`); `consume_fascia(_, None)` (Tentative, as every
   `*_end`), each committing what it consumed; the stash refusing is `stash-refused`, the disk
   refusing is `Error::IO`. Then `persist()`, which finds nothing to write on this base.
5. **Database**, in the check's transaction: `record_broadcast` (BDK apply and persist, change
   promoted, inputs spent), the batch transfer `WaitingConfirmations` (the existing-row branch of
   `update_or_save_transfers`, called directly, so the save branch cannot run), a drain's
   reservations deleted.
6. **P7**, then upstream's asset and media checks, on the same runtime.
7. `go_online`: `update_backup_info`, commit; then `.vss_restored` removed, the runtime dropped,
   `trigger_auto_backup`, the report stored.

What a failure leaves: up to step 3, nothing beyond what the scan persists to BDK, as upstream
today. From step 4 to the commit, the database rolled back and the stash (and BDK) ahead of it,
which is S2's state: the next `go_online` plans the same completion and skips the bundles. The
reverse order would commit `WaitingConfirmations` with change the stash cannot spend.

On rgb-ops 0.11.1-rc.11 the stash is durable at step 4 before `persist()`: rgb-lib loads the
stock with `autosave`, and a successful `consume_fascia` stores index, state and stash in its own
commit (nonasync `Persisting::store`, stash last); `persist()` then finds nothing dirty. The
durability point is that commit, fascia by fascia; a write the disk refuses there was a panic
(rgb-ops' rollback of its in-memory providers is `unreachable!()`) and is `Error::IO` since the
atomic stock store ([§7](#7-the-rgb-stock-on-disk-cc-101)), which also holds the stash back while
the index or the state is not stored, so a bundle the stash holds is one the other two hold
(the skip checks the index as well, for a set left otherwise). `persist()` stays for a base whose
stock does not store at each commit. A second consume of the same fascia changes nothing on
that version (`PubWitness::merge_reveal` returns early on equal TXIDs, bundles and assignments
are sets), so skipping known bundles is visible only when the file and the stash differ; it is
there for a base whose stash does not merge (`-bfa`).

The report costs the completing `go_online` the lookups of step 2 and the writes; a
`go_online` with nothing to complete runs upstream's check.

### What the bridge does (era_rgb, API 16)

API 13, 14 and 15 are taken (the bridge's rounds 7, 8 and 10, the last `91285938`), so this
ships as 16, together with the errors of [§7](#7-the-rgb-stock-on-disk-cc-101), or the next free
number at merge under the bridge's merge rule (`3e48ae71`).

- `connect` sets `complete_unrecorded_spends: true` in its `OnlineOptions` literal (which has no
  `..Default::default()`, so a new field cannot be forgotten), pinned with
  `skip_consistency_check: false` by a guard test.
- `go_online` / `go_online_direct` return a report built from `wallet.completed_spends()` read
  under the same lock right after `go_online`. A non-empty report is a mutation
  (`record_mutation`): the seal debt and the VSS marker go up, an upload follows under
  `AfterEachMutation`.
- Errors: `UnrecordedSpend` → not retryable, `UnrecordedSpendUnseen` → retryable, each with its
  own code; `Inconsistency` gains the reason from `inconsistency_reason()`. The bridge's errors
  carry the reason code and the batch index, **not the TXID** (design question 4: the app's
  device-log rule, a TXID links to whom the user pays); the report keeps the TXID, which the
  host needs to clear its keeper and which it does not log.
- The drain guard in `drain_to_end`, on `vanilla_tx_record(txid)`: a pending Drain proceeds; a
  Drain no longer pending (recorded by a broadcast or a completion) returns `Ok(txid)` without
  calling rgb-lib, so a re-send of the same signed PSBT after a completion is not reported as a
  failure; no record at all (a `dry_run = true` PSBT, an aborted drain) is refused with its own
  code (`DrainNotPending`), not `InvalidPsbt`, which the app reads as a faulty device reply.
- A cancel (`fail_transfers` of one send) whose `signed.psbt` holds another TX or does not parse
  now fails it (`1ee1caf`) rather than answering `InvalidPsbt`; `refresh` still reports that
  `InvalidPsbt` for the transfer.
- The bridge never calls rgb-lib's public `sync`: its orphan reconcile would mark the inputs
  spent without the transition or the status, and the completion would never see them (a guard
  keeps it that way).
- For the app (T1.3, design rev. 6.1, accepted): R5, the offline tail of opening a wallet, never
  cancels a pending Drain whose PSBT was handed out to the device (the app's keeper holds it as
  handed out or signed), nor one the keeper does not know (a VSS copy from another phone, where it
  may have been handed out): such a Drain is left to `goOnline`, which completes it if it
  reached the network, and is cancelled only on proof (T3.3). A Drain the app's handout rule
  withheld (its `drain_to_begin` succeeded, the seal before the handout did not, so the PSBT
  never left the app) is cancelled with `abort_pending_vanilla_tx` at once: at the failed
  handout, and by R5 if the process died first. Other vanilla TXs are cancelled when the keeper
  holds no handed-out or signed record of them.
  `BroadcastFailed` / `IndexerUnavailable` from `sendEnd` / `drainToEnd` mean "outcome unknown":
  keep the signed PSBT, offer no cancel. The retry cadence of `UnrecordedSpendUnseen` is the
  app's (design question 3).

### Tests

`cargo test --locked --lib --features esplora,vss -- wallet::test::scripted_chain::
wallet::test::unrecorded_spends::` (4 tests, and 29 for CC-99 beside §7's two; offline, in
`era.yml`, about 50 s).

**The scripted chain** (`f82b1c1`, `src/wallet/test/scripted_chain.rs`) replaces the regtest
recordings the design planned (the Docker daemon was not available, and recordings go stale with
every esplora-client upgrade): one local HTTP server answers the Esplora API rgb-lib and BDK use
from an in-memory chain the test funds and mines, and the RGB proxy's JSON-RPC. A fault answers
the next matching requests with a chosen status, after the request took effect if asked; a
withheld TX is unknown to the indexer until released; a TX can be evicted. Every request is
logged, and one no route answers fails the test. Wallets under test sign, broadcast, issue and
settle for real (signatures are not checked). Its self-tests reproduce S1 as upstream meets it.

**Wallet level** (`src/wallet/test/unrecorded_spends.rs`):

| # | Case | Asserts |
|---|---|---|
| W1 | donation S1, in the mempool and in a block | the report; one status lookup; `WaitingConfirmations`; the stash witness is the signed TX; the next `go_online` completes and asks nothing; `go_offline` drops the report; settles; the change is spent by the next send; balances |
| W2 | `refresh` S1 of an ACKed send | the report (`WaitingCounterparty`, not a donation); no RGB proxy request during `go_online`; unsigned witness; settles |
| W3 | two ACKed sends broadcast by one `refresh`, both answers lost | both completed in one `go_online`; the next one asks nothing |
| W4 | drain S1 | the report (no batch); nothing pending; no colored UTXO left |
| W5 | S3: copy after `send_begin`, donation and ACKed send | completed as `Initiated`; unsigned witness; settles with the right balance |
| W6 | VSS marker | kept through `MOCK_FAIL_BEFORE_COMPLETION_COMMIT`, removed by the completion that commits; kept, with `RestoredBackupInconsistent` and its reason, when refused |
| W7 | stash ahead of the database: `MOCK_SEND_END_CRASH`, and a completion failed before its commit | completed; stash files byte-equal to a copy of the wallet whose `send_end` went through |
| W8 | option off | `Inconsistency` with exactly upstream's details, no reason, no status lookup, database and stash unchanged |
| W9 | `skip_consistency_check` | no lookup, no completion; the report replaced by the next `go_online` |
| | a `go_online` failing on a refused forwarder URL, or on a new indexer that does not answer (`74664d9`) | no report left; in the second case the previous session still works |
| W10 | refusals: copy before `send_begin`, a chain of two sends, a `Failed` donation later broadcast, fascia deleted, lookup 502, TX evicted after a local apply | the error and reason; database and stash digests unchanged |
| W11 | `try_complete_batch` with another send's `signed.psbt` (`27d2a5a`) | a per-transfer `InvalidPsbt`, no `POST /tx` |
| | that send failed on request (`1ee1caf`); the same with its TX broadcast by another copy of the wallet; a send whose `signed.psbt` is empty | `Failed`, no `POST /tx`, the asset spendable again; `CannotFailBatchTransfer`, still `WaitingCounterparty`; every refresh `InvalidPsbt`, then `Failed` as the first |
| | a completion | `backup_info()` goes back to true after a backup |
| | a fascia whose second bundle the stash refuses | `stash-refused`, database and stash digests unchanged |
| | two spends in one plan, the stash refusing either (`adb712a`) | the database unchanged, both `Initiated`; the earlier spend in the stash only when it came first; the same refusal again; once the fascia is whole, both completed and settled |
| | the wallet's `rgb/` read-only, a real write failure (`3b856ac`) | `Error::IO`, database and stash unchanged, `Initiated`; once writable, completed and settled |
| | the lookup answered by an error page naming the forwarder's secret (`3768cc6`) | the error to the caller as it came; the log line without its text |
| | another install spending the same coin (CC-27), mempool and mined (`fe22585`) | `spender-not-recorded` naming its TX; the view holds the canonical spender only, BDK's graph both |
| | a completion beside a pending, unbroadcast send | only the lost spend completed; both settle |
| | after a backup, `go_online` with the option off and on, nothing to complete (`2b44c76`); options without the field | `backup_info()` stays false; the option off |
| | an unreadable fascia; a media row without its file (`690f041`) | `Error::IO`, then completed; the media inconsistency after the completion, nothing committed |
| | bundles the stash holds, with a damaged fascia file | completed (the stash is not consumed again) |
| | `vanilla_tx_record` of a dry run, an aborted, a pending, an unanswered and a completed drain (`4093572`) | none, none, pending, pending, recorded; `drain_to_end` of the same signed PSBT after the completion answers with its TXID |

**Planner table** (`plan` over real wallet states, each case changed in a transaction rolled
back and a copy of the transfer files, the indexer's answers given by the case): two donations
completed in TXID order with their signed witnesses; the witness never BDK's copy (a same-TXID
copy with other witness data); a donation without `signed.psbt`; `WaitingCounterparty` with ACK
none, true or false and expired; an unexplained coin (Inconsistency, nothing asked); an
incoming-only record; a later spend unrecorded, and allowed when its row is spent; a failed
lookup, an unknown TX, a failed lookup beating an unknown TX, an unknown TX beating a `Failed`
record; `Failed`, `WaitingConfirmations`, `Settled`; two batches and a batch plus a Drain with one
TXID; an inflation and a `CreateUtxos` record; six ways of missing or unreadable data; a fascia
and a `signed.psbt` of the other spend (donation and not), an asset the fascia does not move, an
input coloring outside T; the moved-along asset; a drain's reservations short, released, and of
a `SendBtc` record; and P7 on a plan that covers nothing.

**Mutations**: 37 in the first round, each failing a test. Through the mutation script, 34: P2's uniqueness dropped; an unknown TX read as known and a failed lookup read as unknown; P4 dropped; the witness, input-coloring, asset, transition-kind and `signed.psbt` TXID checks dropped; `Failed` accepted; incoming records accepted; the no-canonical-spender and spender-not-recorded checks dropped; `UnrecordedSpendUnseen` after `UnrecordedSpend`; the consume skipped; the status write, the reservation release, `persist()`, `update_backup_info` and P7 dropped; the runtime persisting on drop again; the option always on; a donation witnessed by BDK's copy of T, or by the unsigned TX; `record_broadcast` without the spent marks; the VSS marker removed before the commit, or kept after it; the bundle skip dropped; the report not reset, or not stored; the drain's three checks dropped; `27d2a5a`'s comparison dropped. By hand: `persist()` moved after the commit (two files), and two of `vanilla_tx_record` (always pending, never found). `persist()` dropped and moved were killed only through `MOCK_STASH_PERSIST_FAIL`, a hook inside `persist()`, which the review of these commits found fires where production never writes; the hook went (`3b856ac`), and both are equivalent mutants on this base. After that review, 8 more, each failing a test: the report reset where it was, the lookup's text logged, spenders taken from BDK's full graph, the divergence counting rows that do not exist, every `go_online` asking for a backup, the option on by default, every I/O error read as missing data, and the checks skipped after a completion. After the verification of those fixes, 1 more: the `InvalidPsbt` arm of `try_fail_batch_transfer` dropped (all three tests of `1ee1caf`).

### Deviations from the design

- **The check's runtime is kept, not re-taken.** The design moved `rgb_runtime()` after the
  wallet-specific checks for every `go_online`; the fork passes the check's runtime, taken where
  upstream takes it, to the completion (`&mut`). Without the option the order of the lock, the
  scans and the drop is upstream's.
- **`colored_divergence` is not extracted from the singlesig check.** The check stays upstream's
  text: its details print a `HashSet` difference, which a shared `BTreeSet` would change. The
  completion computes the same set.
- **No new broadcast or refresh hooks.** S1 comes from the scripted indexer (a relayed `POST
  /tx`, its answer and the lookup lost), for `send_end`, `refresh` and `drain_to_end` alike; a
  `refresh` killed before its commit differs from its S1 only by the ACK rows, which the
  completion does not read. S2 of a donation uses the existing `MOCK_SEND_END_CRASH`. New hook:
  `MOCK_FAIL_BEFORE_COMPLETION_COMMIT` (and `MOCK_STASH_PERSIST_FAIL`, removed after the review
  of these commits: it fired where production never writes).
- **No regtest recordings**: the scripted chain (above). CC-27, two installs of one wallet, runs
  offline on it (`fe22585`). Of §5.4 of the design only the regtest run of the whole upstream suite
  with the option on remains: its harness needs Docker.
- **The stored fascia carries the unsigned TX**, not the TXID only (`rgb_commit` builds
  `PubWitness::with(unsigned_tx)`); every statement about the stash's witness says so.
- **From the review of the design**: the runtime stops persisting on drop before the first
  consume and a consume failure is `stash-refused`; `.vss_restored` goes after the commit;
  `Inconsistency` carries a parsable reason and both new errors the batch index; bundles the
  stash holds are skipped on every base; the drain guard's three outcomes and the R5 rule above
  replace the note-based rule; tests read the stash files.
- **From the review of these commits** (`74664d9` to `690f041`): the report is cleared before
  anything in `go_online` can fail; a refused completion logs the class of a failed lookup, not
  its text; a stash refusal is documented as keeping the plan's earlier spends, which is S2's
  state, and pinned; P1, the flag-off contract, a retryable I/O error and the checks after a
  completion are pinned. A check-all-first pass before the consumes, which would make a stash
  refusal write nothing, was not added: rgb-ops' own pre-check (`check_opid_commitments`) covers
  one of the ways a consume fails, so the guarantee would still not hold for the others.
- **From the verification of those fixes** (`1ee1caf`, `df04de0`): a send whose `signed.psbt`
  will not be broadcast, because W11's guard refuses it or because it does not parse (upstream's
  own lock, for a file `send_end`'s unsynced write left empty), could not be failed until it
  expired (the invoice's expiration, or the bridge's hour), its coins reserved, and the user's
  cancel answered `InvalidPsbt`. `try_fail_batch_transfer` now counts that `InvalidPsbt`, for an
  outgoing send in `WaitingCounterparty`, as no change, next to the forwarder's refusal: the send
  is failed unless its own TX is on chain (`CannotFailBatchTransfer`). In that refresh both
  sources come before any broadcast. `plan_send` still refuses such a batch as `record-mismatch`
  at `go_online` when its TX is on chain (another copy of the wallet broadcast it): that file is
  what `try_complete_batch` would broadcast, a deliberate stop. The rustdoc of `UnrecordedSpend`
  and of the option no longer says a refusal writes nothing.
- **`vanilla_tx_record`** (`4093572`) is new: the bridge's guard cannot tell a recorded drain from
  an unknown PSBT through `list_pending_vanilla_txs`, which lists pending ones only.
- **`Failed` with T on chain stays terminal** (design question 1; the lead's decision of 28.09).

### Carrying it

Checked with `git merge-tree --merge-base=62a8c3a <tag> 690f041` (2026-09-28; not compiled); the
verification's `1ee1caf` and `df04de0` add no conflict (the arm in `try_fail_batch_transfer`
merges cleanly on both tags, the rustdoc lands inside the `Error` and `OnlineOptions` hunks listed below):

- **Both tags**: one new conflict in `src/wallet/mod.rs`, the `pub use objects::{..}` list, where
  UTEXO's `AssetBFA` meets `VanillaTxRecord`: keep both.
- **`v0.3.0-beta.34-bfa`**: otherwise the new field joins the existing `eth_rpc_url` hunks
  (`OnlineOptions`, the UDL dictionary, both examples, `test_go_online_options`).
- **`v0.3.0-beta.43-bfa`**: `broadcast_psbt` merges cleanly, its `release_reserved_txos` landing
  inside `record_broadcast`, so a completion releases reservations as their broadcast does (the
  drain's own delete in `apply` is then a no-op). `go_online_impl`, the consistency check,
  `singlesig.rs` and the new module merge cleanly. New conflicts: `Error` and the UDL, where the
  two errors land next to UTEXO's `UnsafeTransferHistory` / `UnexpectedTransfer` (keep both).
- **To add on `-bfa`**: refuse, as `external-operation`, a batch whose directory holds
  `COLOR_PREPARE_FILE`, `PREPARE_BATCH_FILE` or `STASH_CONSUMED_FILE`, or whose TXID a `psbt_ops/`
  operation names (`-bfa` `rust_only.rs`): those belong to UTEXO's `consume_transfer_fascia` /
  `psbt_op_apply`, which keep their own `STASH_CONSUMED` marker. `PREPARE_BATCH_FILE` is the one
  that lasts: `persist_color_prepare_batch` writes it for color-prepare and PSBT-operation batches
  alike, and `psbt_op_apply` removes `STASH_CONSUMED` after its commit. Refuse the same way UTEXO's
  bridge batches: on `v0.3.0-beta.43-bfa` `bridge_begin_impl` saves an outgoing batch (main
  transition Bridge) and reserves its inputs under a `wallet_transaction` of type `RgbTransfer`
  for the same TXID, so today such a spend is refused as `record-ambiguous`; check for them before
  the record count.
- **On `-bfa`, `go_online` asks an Ethereum RPC** (`web3_clientVersion`) of any wallet whose
  schemas include BFA: give the scripted chain's wallets schemas without BFA, or add that route,
  or every scripted test fails on an unmatched request.
- **The stash**: the private rgb-ops of `-bfa` may not merge a second consume; the bundle skip
  covers S2 and a completion that did not commit, whatever it does. W7 and the planner table have
  to pass there, plus one live signet completion on a BFA asset, before T4.6 ships: that base
  cannot be compiled until UTEXO publishes `s-bfa`.

## 7. The RGB stock on disk (CC-101)

`ac6724d` and `7d07452`, after a review of the ERA bridge (2026-09-28) that proved both halves
on real files: a full disk and SIGKILLs mid-store leaving a stock file cut short, and a missing
file "healed" by an empty stock written over the survivors. Then `5f5edb8` to `f82cc6b`, after a
review of those two commits (the same day: one major, the stash stored after a file that failed;
the rest durability, what the error means, and what was pinned). Then `74a2d66` to `f5d6522`,
after a verification of those fixes (no blocker, no major: coverage lost by `5dd72f3`, the kind
kept through the VSS mapping, symbolic links, and pins). This concerns every rgb-lib wallet, not
only ERA's: to propose to UTEXO.

### Why

rgb-ops 0.11.1-rc.11 keeps a wallet's RGB state in `rgb/stash.dat`, `state.dat` and `index.dat`
(`persistence/fs.rs`, `FsBinStore`). Upstream rgb-lib used it as it is:

- **Stores are in place.** `strict_serialize_to_file` does `File::create` (truncating) and
  writes field by field, unbuffered, with no sync and no rename (rgb-strict-encoding
  `traits.rs:397-406`). Every RGB mutation stores (autosave), so a full disk or a kill during any
  of them leaves a file cut short. The review measured it: on a 40 MB volume with one block free,
  `stash.dat` left at 5120 of 5170 bytes; 78 of 200 SIGKILLs of a process storing left
  `stash.dat` at 3, 8 or 395 bytes. Every later open then failed with `Io{unexpected end of
  file}`, which a host reads as a passing I/O failure.
- **A failed store panicked.** In `RgbRuntime::drop` (`expect("unable to save stock")`), in the
  new-stock branch of `load_rgb_runtime`, and inside rgb-ops itself: a store that fails at a
  commit makes `store_transaction` roll back the in-memory providers, whose rollback is
  `unreachable!()`. In the app a panic poisons the bridge.
- **A missing file became an empty stock.** When any one of the three files was `NotFound`,
  `load_rgb_runtime` built `Stock::in_memory()` and `make_persistent` wrote all three: deleting
  `index.dat` and opening rewrote a 7979-byte `stash.dat` as 4432 bytes, its contracts gone, and
  the app would then seal that over its good seal.

### What changes

**The store** (`ac6724d`, `5f5edb8`, `1a078a7`, `src/stock_store.rs`). `StockStore` is the
stock's persistence provider instead of `FsBinStore`, over the same three files, read the same
way:

- Each file is written whole to `<name>.new` in the same directory, synced (`sync_all`, which is
  `F_FULLFSYNC` on Apple platforms), renamed over the file, and the directory synced. On disk a
  file is always a version that was written completely: the previous one until the rename, the
  new one after. A store whose bytes equal the file's writes nothing (every rgb-ops transaction
  starts with such a store of the unchanged state), except after a directory sync of that file
  that failed: then it syncs the directory, and only that clears the failure (`1a078a7`; before,
  it cleared it with the rename possibly still out of the disk's directory).
- A store that fails is never returned to rgb-ops, whose answer would be the panic above. It is
  kept, per file, until a later store of that file succeeds, and `RgbRuntime` checks it after
  every call that can store (`consume_fascia`, `accept_transfer`, `import_contract`,
  `import_kit`, `store_secret_seal`, `update_witnesses`, `upsert_witness`) and after `persist()`,
  and returns it: `InternalError::StockNotStored`, which becomes `Error::IO`. The file keeps its
  previous version, and the operation fails as it would on any I/O error. Nothing on that path
  panics any more: the drop's store cannot fail (nobody is left to tell there; the call that
  dirtied the stock reported its own failure), the new-stock branch returns the error, and the
  import of an issued contract (`import_and_save_contract`, and the multisig issuance) returns it
  instead of its `expect`. The CC-99 completion reads it as the retryable I/O error it is, not as
  `stash-refused`.
- **The stash is held back** (`5f5edb8`). rgb-ops chains a commit's stores (index, state, stash)
  so that a failed one stops the rest; with failures no longer returned to it, the chain went on,
  and a failed `index.dat` or `state.dat` store was followed by a stored `stash.dat`. The set then
  loaded with the stash ahead of the file that failed, the CC-99 completion skipped the spend as
  held, and its change could never be spent: the next send failed on a bundle "absent in the
  index", or panicked in input selection. The stash is now written only while no other file's
  latest store failed; otherwise it is recorded as failed too ("held back: index.dat is not
  stored") and reported the same way. The stash is never ahead of the index or the state, so a
  spend whose index or state did not reach the disk is consumed again by the next attempt.

**The load** (`7d07452`, `1a078a7`, `aee685d`, `3889751`, `stock_store::open_stock`, through
`load_or_create_rgb_runtime`):

| On disk | Result |
|---|---|
| all three files | loaded |
| one cut short or empty | `RgbStockDamaged`, kind `cut_short`, naming the file; nothing written |
| one longer than its content | `RgbStockDamaged`, `trailing`; nothing written |
| one whose content does not decode | `RgbStockDamaged`, `undecodable` (or `cut_short`, as it reads); nothing written |
| one not a regular file, or `rgb` not a directory | `RgbStockDamaged`, `not_a_file`; nothing written |
| `rgb` or a stock file a symbolic link | `RgbStockDamaged`, `not_a_file` ("rgb is a symbolic link"); nothing written, nothing behind the link touched |
| one the system refuses to read (permissions) | `Error::IO`, as before |
| some but not all | `RgbStockDamaged`, `missing` ("index.dat missing"); nothing written |
| none, wallet without a manifest (a wallet being created; multisig and MPC wallets) | a new, empty stock, made in `rgb.new/` and renamed into place whole |
| none, wallet with a manifest, or a runtime taken during operation | `RgbStockDamaged`, `missing` ("no stock file for a wallet that was already set up") |

- **New only without a manifest.** `Wallet::new` writes the manifest last, after `setup_rgb`, so
  a wallet with one had its stock made. Multisig and MPC wallets have no manifest and keep
  upstream's rule for "none". A runtime taken during operation (`rgb_runtime()`) never makes a new
  stock (it does not look for a manifest; the message holds for both).
- **A new stock is never seen half made.** It is written in `rgb.new/` next to `rgb/` (each file
  as above), then renamed into place and the wallet directory synced. A kill at any point leaves
  either no stock file in `rgb/` (the next attempt starts again) or the whole set.
- **Leftovers.** A `.new` file a store left behind, and a stale `rgb.new/`, are removed at load,
  under the runtime's lock. Then `rgb/` is synced once (`1a078a7`), before the stock is built on
  it: a kill between a store's rename and its directory sync leaves a rename nothing made
  durable.
- **`rgb` as a file** was `Error::IO` on every attempt, for ever; it is `not_a_file` (`aee685d`).
- **Symbolic links are refused** (`3889751`), `rgb` or a stock file, as "... is a symbolic link".
  A linked `rgb` loaded before `aee685d`, but both backup walks (`backup.rs`, `vss.rs`) go
  through the wallet directory without following links, so it was backed up empty and the
  restore was refused for a missing stock (reproduced by the verification); a store renames its
  new file over a linked file, which replaces the link. rgb-lib and the ERA app never make
  either. The leftover cleanup checks `rgb` without following it, so nothing behind a link is
  removed or synced.
- **The lock.** A load that fails now releases the runtime's lock file. It stayed, and every later
  load waited out `LOCK_FILE_TIMEOUT_SECS` (an hour outside tests) before "unreleased lock file".

**The error** (`aee685d`, `5dd72f3`, `50a470b`). Its final shape:

```rust
Error::RgbStockDamaged { details: String, kind: String }
// "The wallet's RGB state cannot be opened ({kind}): {details}"
// UDL: RgbStockDamaged(string details, string kind);
```

- `details` names the file and how: "stash.dat is cut short", "index.dat is longer than its
  content", "state.dat does not decode: ...", "state.dat is not a file", "rgb is not a
  directory", "rgb is a symbolic link", "index.dat missing", "no stock file for a wallet that
  was already set up".
- `kind` is one of `missing`, `cut_short`, `trailing`, `undecodable`, `not_a_file`, the codes of
  the exported `RgbStockDamage` (`code()`, `from_code()`); `Error::rgb_stock_damage()` reads it
  back from this error and from the restored-backup form below. It is a hint for diagnostics, **not a
  way to tell a format change from damage**: random corruption reads as `cut_short` too, and
  `cut_short`, `trailing` and `undecodable` are all what intact files of another rgb-ops format
  look like.
- **It is a refusal to open, not a signal to discard the files.** Nothing was written. The files
  may be intact: a stock written by another version of rgb-ops (an app downgrade reading a newer
  format, a base with other RGB libraries such as the `-bfa` carry) fails the same way, and the
  interrupted writes it would otherwise point to no longer happen (stores are atomic). A host
  keeps the directory as it is, and replaces it with a backup only when it can rule out a format
  change, for example because that backup was written by the same build, or when it keeps the
  directory's content to go back to (what the ERA app does, below). There is no integrity
  check: a file that still decodes is accepted as it is (see [Not done](#not-done)).
- **After a VSS restore it is reported against the backup** (`5dd72f3`, `50a470b`, `vss`
  feature). While the restore marker (`.vss_restored`) is in the wallet directory, `setup_rgb`
  returns it as

  ```rust
  Error::RestoredBackupInconsistent { details: format!("RGB state ({kind}): {details}") }
  // e.g. "RGB state (missing): index.dat missing",
  //      "RGB state (trailing): stash.dat is longer than its content"
  ```

  with the marker kept. The details always start `RGB state (<kind>): `, the kind one of the five
  codes; `Error::rgb_stock_damage()` returns it (and `None` for a restored backup that failed the
  consistency check, whose details are upstream's), and `inconsistency_reason()` is `None`. A
  backup torn this way reports the same error whether it was encrypted (which keeps the
  manifest, so it was refused as `RgbStockDamaged`) or plaintext (manifest stripped, so an empty
  set got an empty stock and failed the consistency check at `go_online`, as upstream).
  `RestoredBackupInconsistent` can therefore come from `Wallet::new` as well as from `go_online`.
  From `Wallet::new` it is **not a verdict on the backup**: `trailing`, `undecodable` and
  `cut_short` may be intact files of another rgb-ops format, and then every newer backup fails
  the same way, so the variant's message ("likely stale ... restore a newer backup") does not
  apply; the rustdoc says so. `5dd72f3` dropped the kind here, which left a host only prose to
  tell the two apart; `50a470b` carries it. A file restore writes no marker and keeps
  `RgbStockDamaged`.

The ERA bridge takes both with CC-99's report (§6) in era_rgb API 16.

**What the ERA app does with a wallet it cannot open** (T1.3, design rev. 6.1, accepted). The
app keeps an rgb-lib wallet directory open only between unlock and lock; the rest of the time
the wallet exists as its seal, rgb-lib's own encrypted `backup` of the directory under a key
derived from the app's data key, taken at lock and after mutations. The handout rule seals
before a PSBT or an invoice leaves the app, so every spend record and every handed-out invoice
secret is in the seal. "Cannot open" is `RgbStockDamaged` (as the bridge's `WalletDamaged`, or
`RestoredBackupInconsistent` with the `RGB state (` prefix) and every other deterministic
refusal of `open` but a missing or already open wallet (a database migration this build lacks,
a panic, a mismatched setting), plus the breaker below and an I/O error that lasts two starts while a
seal exists.

- **Never deleted, never left open in plaintext.** The directory goes to an encrypted
  quarantine: streamed as tar through AES-256-GCM under a key of its own, synced, and only then
  removed; the same content is never written twice. Nothing deletes a quarantined directory
  but "Clear all data".
- **Then a restore from the seal, with no pre-check of the build that wrote it** (no seal: the
  wallet is blocked, its exits a VSS restore or starting empty). Nothing is
  lost by trying (the directory is in quarantine, and it holds nothing that cannot be recovered
  beyond the seal), and a restored copy that does not open is itself the sign of a format
  change. A check by build id would not have caught the one dangerous case, and would have
  blocked the wallet after every app update.
- **If the restored copy does not open either,** it is deleted (it came from the seal) and the
  app reads which build wrote the seal (version and build number, e.g. `1.20.0+92`, from its own
  note in the copy, not through rgb-lib). **The same build**: no format change is possible, so
  the seal is damaged; it is set aside (renamed, never deleted) and the wallet is blocked with
  the exits of a wallet without a live seal (a VSS restore, or starting empty, both confirmed by
  the user). **Another build**, or a note it cannot read: a format or schema change is possible;
  the seal stays, and the wallet stays blocked until a newer app version, which tries once.
- **A process death inside `open`** is bounded by the app's breaker. rgb-strict-encoding
  allocates what a length in the data asks for, so a flipped bit in `stash.dat`, or a stash of
  another format, can abort the process on the allocation: no panic to catch, no error. The
  breaker counts, per wallet and app build, the opens that never returned (on disk, around
  each `open`): at most 3 per app version (version and build number). One changes nothing, two
  send the directory to quarantine, and the third, on the seal's copy, is decided by the seal's
  build as above. A copy restored from VSS gets at most 2, then it is deleted (the server's copy
  stays) and the wallet goes back to its block.
- **A process death outside `open`** is not bounded by the app: a payer's consignment decoded in
  `refresh` (CC-103) can kill it on every unlock, and the breaker does not see it. Only the
  decoder patch closes it (rgb-strict-encoding allocating no more than the bytes left; not in
  this series), and that patch gates Receive in the app: there are no incoming transfers
  before it.

### What does not change

- **The files.** Same names, same format. What is pinned, and nothing beyond it:
  `the_files_are_the_ones_rgb_ops_writes` stores one stock here and through `FsBinStore` and
  compares the bytes (this build on both sides); `d82e21a_file_backup_still_restores` opens the
  wallet of the `d82e21a` fixture, whose `rgb/` the rev the app pinned before wrote, with
  `Wallet::load`, checks its three files byte-equal afterwards, and that each, decoded,
  serializes back to its own bytes (`f82cc6b`); `a_stock_over_64_kib_is_stored_and_loaded_back`
  stores and loads a stash over `u16::MAX` bytes (the length is read and written as `FsBinStore`
  does, up to `u32`). One older rev is pinned, not every one. Backups and restores carry the same
  files.
- **Not atomic as a set.** Each file is, the three together are not: a failure or a kill between
  their stores within one commit can leave them at different versions. The stash is never the
  newest: a kill follows rgb-ops' order (index, state, stash), a failure the hold-back above. The
  set still loads, and re-running the operation merges what is already there. CC-99's bundle
  skip counts a bundle as held only when the stash and the index both know it (`5fdfa17`, by
  bundle ID, which commits to its input map): for a set left otherwise (written before the
  hold-back, or by another hand) the bundle is consumed again, which rgb-ops merges. The state is
  not checked: with the hold-back, a stash that holds the bundle had the state stored with it.
  Making the set atomic would take a generation scheme over the directory, with its own format;
  not done.
- **Cost.** Each changed file costs a file sync and a directory sync. Stores of unchanged data
  are skipped, which halves the writes of a transaction; they sync the directory only after a
  failed directory sync of that file. A load syncs `rgb/` once. The scripted-chain suites, which
  fund, issue, send and settle, took 26 s for 26 tests after and 24 s for 24 before, on a Mac.

### Tests

`cargo test --locked --lib --features esplora,vss -- stock_store::` (22 tests and the kill
test's child, offline), five scripted-chain tests in `wallet::test::unrecorded_spends`, and
`d82e21a_file_backup_still_restores` in `wallet::backup::tests`; all three sets run in `era.yml`.
Failures are injected per file through test-only hooks (`STORE_FAILURES`: half of the new file
written, the process gone before the rename, the directory sync failing), and the steps of a
store are recorded (`STORE_EVENTS`).

- A store stopped half way, and one stopped before the rename (then a planted `.new` cut short,
  as a kill leaves it): the file byte-equal to the previous version, no `.new` left, the stock
  loads, `Error::IO` returned, no panic; a stale `rgb.new/` removed at load.
- A failure lasting until the file is stored again, the next store catching up with memory; a
  directory sync failure reported, and made up for by the next store of the same bytes (which
  records a directory sync); a load recording the sync of `rgb/`; the order write, sync, rename,
  sync directory, and nothing written for unchanged data; `import_kit` and `upsert_witness`
  reporting a failed store; a stash over 64 KiB (6000 secret seals) stored and loaded back whole.
- **Kills**: a child process runs stores in a loop and is SIGKILLed 24 times at spread delays;
  after each, the stock loads whole (on the run recorded here 4 of the 24 kills left a `.new`
  behind, so they did land mid-store).
- Each file missing; cut short, empty, longer than its content, a directory: refused with its
  kind and message, the other files byte-unchanged, whether a new stock is allowed or not, the
  lock released; content that does not decode, as it reads (`f5d6522`): 64 bytes of `0xff` are
  `undecodable` for `stash.dat` and `state.dat` and `cut_short` for `index.dat`, a `stash.dat`
  with its first byte changed `undecodable`; `rgb` a file refused as `not_a_file` either way,
  nothing written; `rgb` or `index.dat` a symbolic link refused as one, with a leftover behind the
  link left alone (`3889751`); an unreadable file an I/O error; no file at all refused with a
  manifest, new without; a new stock that fails half way (a file cut short, a kill before a
  file's rename, before the directory's) leaving no stock file, the next attempt making it whole;
  a wallet reopened without `index.dat` (its stash kept byte for byte) or without `rgb/`,
  refused.
- **The VSS mapping** (`50a470b`): a wallet without `index.dat`, and one whose `stash.dat` has
  trailing bytes, each `RgbStockDamaged` without the marker and, with it,
  `RestoredBackupInconsistent` "RGB state (missing): index.dat missing" / "RGB state (trailing):
  stash.dat is longer than its content", the kind read back by `rgb_stock_damage()` both times,
  no inconsistency reason, the marker kept; a consistency-check refusal reads as no kind. Every
  kind reads back from its code, the five codes pinned, unknown ones refused.
- The CC-99 completion and an issuance whose stash store fails: `Error::IO`, nothing committed,
  the next attempt succeeds.
- **The hold-back** (the review's six probes, `a_stock_file_that_fails_to_store_keeps_the_stash_behind`):
  each of the three files failing to store, during a completion and during `send_end`:
  `Error::IO` with the stash unchanged, the next `go_online` completes the spend, it settles, and
  its change is spent.
- **The index check** (`a_bundle_the_index_does_not_know_is_consumed_again`): an `index.dat` from
  before a completion that consumed and did not commit put back; the next `go_online` consumes
  again, the spend settles, and its change is spent.
- **The format**: the `d82e21a` wallet as above.
- **The asset check** (`74a2d66`, `the_asset_check_refuses_a_stock_without_the_asset`): an
  issuer's `rgb/` replaced by a fresh wallet's, a whole stock without the asset: `go_online`
  refused with upstream's `Inconsistency` "DB assets do not match with ones stored in RGB", with
  the completion option and without. `5dd72f3` had moved the only tests of that branch to the
  refusal at `Wallet::new`, and `if false && ...` on it survived `era.yml`.
**Mutations**: 26 in the first round, each failing a test. The store: written in place (no `.new`), the file sync or the directory sync dropped, the rename before the sync, a failure not kept, never cleared, or returned to rgb-ops (the test process aborts on the panic this section removes), unchanged bytes rewritten, a `.new` left after a failure, `RgbRuntime` or `persist()` ignoring a failure, the failure mapped to `Internal`, the completion reading it as `stash-refused`, an issuance panicking on it. The load: a partial set treated as none, cut short, trailing data or undecodable read as `IO`, an unreadable file as damaged, a directory as a file, a new stock with a manifest or where not allowed, a new stock made in place, a stale `.new` or `rgb.new/` kept, the lock kept after a refusal. A mutation of the kill test's own subject (writing in place) fails the deterministic tests; the kill test is evidence, not a mutation killer. One mutant per wrapper of `RgbRuntime` was not run for `accept_transfer` and `update_witnesses`, which need a consignment or a resolver; they route through the same `stored()`. After the review, 6 more, each failing a test: the hold-back dropped (the six probes), the index dropped from the skip, the directory sync of a pending failure dropped, the load's sync of `rgb/` dropped, the VSS mapping dropped, and a `u16` length limit in place of `u32`. After the verification, 8 more, each failing a test in `era.yml`'s set: the asset check skipped, a decode error read as `cut_short`, `from_code` answering `None` (the two together had survived), the kind dropped from the restored details, `rgb_stock_damage()` answering `None` for them, the symbolic-link arm dropped, the check following the link (`metadata`), and the cleanup following it (`is_dir`).

### Carrying it

Checked with `git merge-tree --merge-base=62a8c3a <tag> 7d07452` (not compiled):
`v0.3.0-beta.34-bfa` adds no conflict; `v0.3.0-beta.43-bfa` adds one, in
`From<InternalError> for Error`, where UTEXO maps `UnknownContract` to `AssetNotFound`: keep both
arms. The review's commits (`5f5edb8` to `f82cc6b`) add none on either tag (the same files and
the same conflict hunks as at `35186ad`, 2026-09-28), nor do the verification's (`1ee1caf` to
`df04de0`, against `fd18e92`: new text only inside the `Error` and `OnlineOptions` hunks already
there). Then:

- **Run the format tripwire first**: `d82e21a_file_backup_still_restores`. It is the check that
  the base's rgb-ops reads the stock the app's wallets hold. If it fails on a decode (the `-bfa`
  tags patch rgb-ops), every wallet the app has would be refused as `RgbStockDamaged` after the
  update, its files intact: that needs a migration of the stock before the carry ships, and no
  host may answer it with a restore. The same goes for a wallet restored from VSS after the
  carry, whose refusal comes as `RestoredBackupInconsistent` "RGB state (undecodable): ..." (or
  `trailing`, `cut_short`): not a stale backup, and every newer backup the old build wrote fails
  the same way.
- **Two upstream tests expect what this changes** (`5dd72f3`, `74a2d66`, `50a470b`):
  `go_online::consistency_check_fail_asset_ids` and
  `vss_e2e::backup_gaps::inconsistent_restored_backup_returns_dedicated_error` remove `rgb/` of
  a wallet that has a manifest, which upstream opened on an empty stock whose consistency check
  failed at `go_online`. Here each keeps one assertion of the refusal at `Wallet::new`
  (`RgbStockDamaged` `missing`; `RestoredBackupInconsistent` starting "RGB state (missing): "),
  then puts a fresh wallet's `rgb/` in place, a whole stock without the asset, and reaches
  `go_online` as upstream does (`Inconsistency` "DB assets do not match with ones stored in RGB"
  for `prefill_2` and `prefill_3`; `RestoredBackupInconsistent` for the restore, the marker kept).
  Both need the regtest or VSS services and were compiled, not run (no Docker on the machine).
  On a new tag take UTEXO's version of each and make the same two changes; an upstream test that
  removes stock files or `rgb/` of a wallet with a manifest and expects an empty stock changes
  the same way. The asset branch itself is also covered offline, in `era.yml`
  (`the_asset_check_refuses_a_stock_without_the_asset`).
- On `-bfa` the private rgb-ops must be checked for the same provider trait, the same `nonasync`
  version and the same rollback behaviour; for the order index, state, stash in a commit's
  stores and the three file names (`STOCK_FILES`), which the hold-back relies on; and every
  `&mut self` method of `RgbRuntime` there must route its result through `stored()`: one that
  does not reports success for a stock the disk refused.

### Not done

- The set is atomic per file only, and the skip does not check the state (above).
- **A symbolic link** in place of `rgb` or of a stock file is refused, not followed (above):
  following it would take teaching both backup walks to follow it too.
- **No integrity check.** A file that decodes is accepted as it is: a changed byte inside a value
  that still decodes loads as another stock, and nothing says so. A checksum needs a format of
  its own (a header or a sidecar file), which the byte-equality with `FsBinStore` above excludes.
- `RgbRuntime::drop` can still panic on the lock file (`expect("should be able to drop
  lockfile")`), and the lock of a process that died stays: the ERA bridge removes it when it
  opens a wallet.
- `WalletManifest::write` renames a temporary file into place but syncs neither (upstream).

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
- `e7bdaa4` conflicts on both `v0.3.0-beta.34-bfa` and `v0.3.0-beta.43-bfa` (checked by
  cherry-picking the whole series in a scratch worktree, 2026-09-27), all of it
  mechanical. UTEXO added `eth_rpc_url` next to the new field: keep both lines in
  `OnlineOptions` / `OnlineData` (`objects.rs`), the UDL dictionary, both examples and
  `test_go_online_options` (keep their `eth_rpc_url` value), and keep both module lines
  in `src/api/mod.rs` (`ethereum`, `forwarder`). They also moved the recipient loop of
  `send_begin` into `parse_recipient`: keep their version and give it the fork's probe (next
  point). Every other hunk of the patch applies as is on both tags, which have the same
  proxy call sites in `online.rs` (six `ProxyClient::new`, one `check_proxy`) and the same
  reject-list call.
- The review fixes after it (`3f0a555` to `87d5e88`) and the VSS commits were checked against
  both tags with `git merge-tree --merge-base=62a8c3a <tag> <this branch>`, the series as one
  diff (2026-09-27; not cherry-picked one by one, not compiled). On `v0.3.0-beta.34-bfa` they
  conflict nowhere `e7bdaa4` does not. On `v0.3.0-beta.43-bfa` there are two more: `Error` and
  the UDL, where `ForwarderRefused` lands next to UTEXO's `PsbtOperationNotFound` (keep both),
  and the loop of `fail_transfers_impl` that fails every expired transfer, which UTEXO already
  changed (a `CannotFail` outcome, prepare batches left alone, `Indexer` and `Network` errors
  skipped, `transfers_changed` set after a success): keep theirs, add `ForwarderRefused` and
  `InvalidForwardTarget` to the errors it skips (`a14cdbb`), and run each attempt in
  `txn.savepoint()`, committed after a success and dropped on every skip, theirs included
  (`1699a6a`). `vss.rs` merges cleanly on both. The third review's commits (`fe7e1b0` to
  `8692b69`) add no conflict on either tag: `try_fail_batch_transfer` (whose chain lookup sits
  next to UTEXO's own lookups for prepare batches on `v0.3.0-beta.43-bfa`), `try_complete_batch`,
  `refuse_consignment`, `DbTxn::savepoint`, `backup.rs` and the fixtures apply as they are, and
  the gap `1051adc` closes is still in `try_complete_batch` there. The fourth review's commits
  (`56e6186` to `a140543`) were checked the same way: on `v0.3.0-beta.34-bfa` they add no
  conflict; on `v0.3.0-beta.43-bfa` the only new ones are comments around
  `TryFailBatchTransferOutcome::CannotFail`, which UTEXO already has with the same meaning and
  the same arm in the single call (keep their variant and doc, drop the fork's), and the bulk
  loop above, where UTEXO's `CannotFail => continue` becomes dropping the savepoint.
  In `parse_recipient`, port the
  probe loop of the fork's `send_begin_impl`: the `refused` variable,
  `self.check_proxy_endpoint(..)` and the match on its result (`InvalidForwardTarget` returns
  at once, the first `ForwarderRefused` is kept), and the check of `refused` before
  `InvalidTransportEndpoints`. The proxy forwarder guard fails while `parse_recipient` still
  calls `check_proxy` (on both tags that is the one extra line); a probe without the refusal
  and target handling only the forwarder tests catch
  (`send_begin_reports_the_forwarders_refusal`,
  `send_begin_refuses_an_endpoint_with_userinfo_or_a_fragment`).
- `5cab571` to `106a00c`, `74664d9` to `690f041`, `1ee1caf` and `df04de0` (CC-99): see [Carrying it](#carrying-it) in
  §6. On both `-bfa` tags they add a conflict in `src/wallet/mod.rs`, on `v0.3.0-beta.43-bfa` also in
  `Error` and the UDL; they need a refusal for UTEXO's prepared and bridge batches, and the
  scripted tests' wallets without the BFA schema.
- `ac6724d`, `7d07452`, `5f5edb8` to `f82cc6b` and `74a2d66` to `f5d6522` (CC-101): see [Carrying it](#carrying-it-1) in
  §7: one more conflict in `From<InternalError> for Error` on `v0.3.0-beta.43-bfa`, two upstream
  tests whose expectation changed, the format tripwire to run first, and the private rgb-ops to
  check.
- If the proxy forwarder guard fires, route the new call through
  `WalletOnline::proxy_client`, `reject_list_client` or `check_proxy_endpoint`, and change the
  expected set in `era.yml` only for a line that is not a call site; do not widen the
  exclusion.
- If UTEXO has merged the layout patch, drop `6ce375e` and its review commits
  `1ed4436` to `e230950`, and check that their field names and defaults match what the
  app sends.
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

Opened on 2026-09-28 as
[UTEXO-Protocol/rgb-lib#104](https://github.com/UTEXO-Protocol/rgb-lib/pull/104) from
branch `era/pr-configurable-keychain-layout`: `6ce375e` alone, cherry-picked onto their
`dev` at `ca5f6b7` (commit `c7202ee`). UTEXO had agreed to take it. We could not compile
`dev` (the `*-s-bfa` mirrors in its `[patch.crates-io]` are private for us); the PR says
so. Once it merges, drop `6ce375e` from the series when carrying it onto the next base.

UTEXO reviewed it on 2026-09-29: fail fast when the account xpubs contradict the coin
types, one struct instead of loose options, the UDL's error order and one `impl` block. The
answer is `c23b140`, `eba4d69` and `c1a1ee8` on the PR branch (pushed the same day, no
rewrite of `c7202ee`), mirrored here as `1ed4436`, `2319444` and `e230950`. Two deviations
from their suggestion, each explained in its commit: the check compares keys (public key
and chain code), not whole `Xpub`s, and `vanilla_keychain` stays outside the struct.

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
