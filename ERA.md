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
| `6ce375e` | Add a configurable keychain layout to singlesig keys | To propose to UTEXO (draft [below](#pr-proposal-for-utexo), not sent) |
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

Later commits that touch only this file are part of the series too. `3f0a555` to `87d5e88`
answer two reviews of `e7bdaa4` ([§4](#4-proxy-forwarder-e7bdaa4)) and go with it wherever the
series is carried; `45c3b39` and `777fe57` are [§5](#5-vss-restore-checks-45c3b39-777fe57).

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

## 2. CI

`.github/workflows/era.yml` (`1d26404`, extended in `f808c7f`, `07e16e5`, `e7bdaa4`, `bef0c03`,
`87d5e88` and `45c3b39`) has two jobs.

- **check** runs on push / PR to `era/**` and on manual dispatch: rustfmt, `cargo check`
  with the app's feature set and with the upstream default on top, a check of the uniffi
  and C-FFI bindings (each its own workspace and lockfile; the uniffi UDL mirrors the `Error`
  enum, and a variant missing there breaks only that build), a dependency guard over the
  app's mobile targets, the HTTP client guard (below), the proxy forwarder guard
  ([§4](#4-proxy-forwarder-e7bdaa4)), the unit tests above plus the REST-client TLS test, the
  forwarder tests, the VSS tests ([§5](#5-vss-restore-checks-45c3b39-777fe57)) and offline
  wallet tests, and the migration crate. Every cargo call is `--locked`: a fresh resolution
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

## 4. Proxy forwarder (`e7bdaa4`)

Reworked after two independent reviews on 2026-09-27, both "pass with issues". The first:
`fafe3bd` (reject list fails closed), `bef0c03` (the route comes only from the wallet),
`ccd23e9` (refusal signal), `8a0255f` (no userinfo or fragment in a target), `cf0afa5`
(forwarder applied before the indexer probe, superseded by `ff6e73a`), `9718cac` (errors never
name the forwarder), `0996972` (rustdoc). The second: `ff6e73a` (a failed `go_online` goes
offline; `go_offline`), `afe4644` (a refusal must echo the forwarder's path), `aa5187d` ("Cannot
change ACK" from the proxy only), `a14cdbb` (a refused send can be failed and does not block
others), `b8bf73d` (a forwarded reject list only from a 200 that holds an opout), `019b917`
and `87d5e88` (tests and CI for what the second review's mutations and probes left open), with
`29a49bb` running the forwarder tests one at a time. This section describes the result.

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
    // None = upstream behaviour; the path may carry a per-session secret
    forwarder_url: Some(format!("http://127.0.0.1:{port}/{session_secret}/rgb")),
})?;
// ... and when the session ends (lock, pause): no request goes out until the next go_online
wallet.go_offline();

// the forwarded twin of rust_only::check_proxy_url (not needed by a wallet that is online)
rgb_lib::wallet::rust_only::check_proxy_url_via_forwarder(proxy_url, forwarder_url)?;
```

- `forwarder_url` must be plain `http` to a loopback IP literal (`127.0.0.0/8` or
  `[::1]`; `localhost` is refused), with no credentials, query, fragment or port 0. A path
  is allowed and used as given. Anything else is `Error::InvalidForwarderUrl { details }`,
  raised by `go_online` **before** it changes anything, so the previous route stays in force.
- **A per-session secret belongs in that path.** A loopback port is reachable by every app on
  the phone, so the forwarder cannot tell rgb-lib from another app by the connection alone. A
  random path segment per session (`/{session_secret}/rgb`), checked by the forwarder on every
  request, can; it also authenticates the forwarder's refusals ([below](#what-rgb-lib-guarantees)).
  rgb-lib keeps `forwarder_url` in memory only (the wallet's `OnlineData`): it is not written to
  the wallet's files, invoices, the database or a backup, not logged, and a failed request's
  error names the URL the request was meant for, never the forwarder's (`9718cac`).
  `OnlineOptions` derives `Debug` and `Serialize`, so its debug output and its JSON carry the
  secret: **the host must not log it**.
- **`go_online` applies the value on every call; a call that fails at the probe of a new
  indexer goes offline.** A refused `forwarder_url` changes nothing. Otherwise the forwarder
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
  an error.
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
    and, when `forwarder_url` has a path, a response header **`X-Era-Forward-Session`** whose
    value is exactly that path (as rgb-lib requested it, e.g. `/6c1f0e…/rgb`). That is the
    forwarder refusing the request. The value of `X-Era-Forward-Refused` is the reason, passed
    through (up to 256 characters, decoded lossily) in
    `Error::ForwarderRefused { target, reason }`, `target` being the `X-Era-Forward-Target`
    value (`ccd23e9`). Anything less is an ordinary answer: a 403 without the refusal header,
    the header on another status, or (with a path) a missing or different session header
    (`afe4644`). The echo is what a relayed target cannot produce: it never sees the path. A
    forwarder at the root has no path to echo, and its refusals go unchecked.
  - **the reject list fails closed**: only a **200** is read as the list, and a 200 whose body is
    not empty must hold at least one line that parses as an opout (plain or `!` allow line);
    anything else is `Error::RejectListService` (`fafe3bd`, `b8bf73d`). Read as a list, an error
    page (or a 203, 204 or 206) holds no opout, and the asset would be validated against an
    empty list. An empty 200 is an empty list: an issuer may publish one.

  For `rgb-proxy`, any other answer is read as JSON-RPC whatever its status, exactly as rgb-lib
  reads the proxy's own; a body that is not a JSON-RPC response is `Error::Proxy`. A reason is
  never read as the proxy's words: `refuse_consignment` matches the proxy's "Cannot change ACK"
  on `Error::Proxy` only (`aa5187d`).

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
  `X-Era-Forward-Refused: <reason>` and `X-Era-Forward-Session: <the request's path>`. The
  reason is a short token of the app's choosing (for example `not-allowlisted`,
  `user-declined`, `consent-expired`) that the app maps to its own message; it is never shown
  raw.
- **Relay the target's status and body as they came.** Never answer with a 3xx. For a failure
  of its own (the backend is down, the target cannot be reached, a pending call is cut short
  on lock) answer a non-2xx status **without** the refusal headers, for example 503, and within
  rgb-lib's timeouts (10 s to connect, 120 s per request): rgb-lib then behaves as for a proxy
  that is down, and the call can be retried.

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
| reject list `GET` | `refresh` and `provide_out_of_band_consignment` (receiving an IFA asset), `send_begin` (sending one) | `ForwarderRefused` | `RejectListService` (a non-200, or a 200 holding text and no opout, included) |

- A refused endpoint next to a usable one is skipped like an unreachable one, so an invoice
  listing an unknown proxy beside a known one sends through the known one without an error.
- `consignment.get` is deliberately unchanged: upstream reads every failure there as "no
  consignment yet", and `fail_transfers` refreshes a transfer before failing it, so an error
  there would make an expired receive impossible to fail for as long as the forwarder refuses.
- **`fail_transfers`** refreshes a transfer first and gives up on its error, as upstream does.
  For a **send still in `WaitingCounterparty`** (nothing broadcast yet), `ForwarderRefused` and
  `InvalidForwardTarget` from that refresh count as "no change", and the send is failed
  (`a14cdbb`): otherwise a lapsed consent would keep it from ever failing. A receive keeps
  upstream's rule (a donation's witness may already be on chain), and so does an outage (the
  recipient may have answered). When failing every expired transfer, a transfer one of those two
  errors still stops (a receive) is skipped and the others are failed; failing that one transfer
  alone returns the error. A failing reject list (refused or not) blocks failing the receive the
  same way, as upstream does when the list is unreachable: that is the cost of failing closed.
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
  `go_online` and `fail_transfers` behaviour. Upstream reads a direct reject list's answer as the
  list whatever its status (a 503 page is an empty list); the fork leaves that path as it is (a
  test pins it). Worth reporting to UTEXO.
- Not routed by this patch, because the host already chooses those URLs: the indexer
  (`indexer_url`), VSS (its server URL), the multisig hub and DFNS (not used by the app).
  On the `-bfa` bases, `OnlineOptions::eth_rpc_url` (BFA validation reads an Ethereum
  RPC) is the same kind of host-chosen URL: point it at the forwarder too. The errors of those
  clients are theirs and have not been checked for URLs.

### Public API the host adapts to

- New: `OnlineOptions::forwarder_url: Option<String>` (`e7bdaa4`); `Wallet::go_offline()`
  (`ff6e73a`); `rust_only::check_proxy_url_via_forwarder(proxy_url, forwarder_url)` (`e7bdaa4`);
  `Error::InvalidForwarderUrl { details }` (`e7bdaa4`), `Error::ForwarderRefused { target,
  reason }` (`ccd23e9`), `Error::InvalidForwardTarget { details }` (`8a0255f`), all mirrored in
  the uniffi UDL, whose build now runs in `era.yml` (`87d5e88`).
- Changed behaviour, with `forwarder_url` set:

  | Call | Was | Now |
  |---|---|---|
  | `go_online` with a new `indexer_url` whose probe fails, a forwarder set before or by the call | error, still online on the previous indexer and forwarder | error, **offline**: old handles get `Offline` |
  | `send_begin` | `InvalidTransportEndpoints` when every endpoint failed its probe | `ForwarderRefused` when a refusal is why; `InvalidForwardTarget` for an endpoint with userinfo or a fragment |
  | `send_end` | `NoValidTransportEndpoint` when no endpoint took the consignment; `Proxy` from `media.post` | `ForwarderRefused` when a refusal is why; `ForwarderRefused` from `media.post` |
  | `refresh` (a transfer's `failure`) | `Proxy` from `ack.get`, ACK, NACK, `media.get` | `ForwarderRefused`; `InvalidForwardTarget` for a stored endpoint with userinfo or a fragment |
  | reject list (`refresh`, `send_begin`, `provide_out_of_band_consignment`) | any answer read as the list | `ForwarderRefused`, or `RejectListService` for a non-200 or a 200 holding text and no opout |
  | `fail_transfers(Some(send))`, the send's ACK poll refused | error, the send not failed | the send failed (`WaitingCounterparty` only) |
  | `fail_transfers(Some(receive))`, a policy error in its refresh | error (`Proxy` for an outage) | `ForwarderRefused` / `InvalidForwardTarget` |
  | `fail_transfers(None)` with such a receive among the expired | error, and nothing failed (rolled back) | that receive skipped, the others failed |
  | `check_proxy_url_via_forwarder` | `Proxy` ("unable to connect to proxy") on a refusal | `ForwarderRefused`; `InvalidForwardTarget` |
  | NACK refused with a reason containing "Cannot change ACK" | the receive failed without its NACK | the transfer's `failure`: `ForwarderRefused` |

### Where it is

`src/api/forwarder.rs` (validation of `forwarder_url`, the client, the target check and the
headers in `Forwarder::request`, `Forwarder::refusal` and `refusal_reason`, `Forwarder::scrub`,
the client-level tests), `ProxyClient::post` / `ProxyClient::call` and
`RejectListClient::get_forwarded` (the one place each builds, sends and reads a routed request),
`WalletOnline::forwarder` / `proxy_client` / `reject_list_client` / `check_proxy_endpoint`,
`go_online_impl`, `go_offline_impl`, `try_fail_batch_transfer`, `fail_transfers_impl` and
`refuse_consignment` in `src/wallet/online.rs`, `Wallet::go_offline`, `utils::check_proxy_routed`,
`OnlineOptions::forwarder_url`, `OnlineData::forwarder`, the three error variants,
`rust_only::check_proxy_url_via_forwarder`.

### Guard and tests

**The proxy forwarder guard** (`era.yml`, "Proxy and reject-list clients only through the
wallet's route"): wallet code reaches an RGB proxy or a reject list only through
`WalletOnline::proxy_client`, `reject_list_client` and `check_proxy_endpoint`, which read the
forwarder `go_online` stored and take no route from their caller. The step lists every
`ProxyClient::new(` / `::new_routed(`, `RejectListClient::new(` / `::new_routed(`,
`check_proxy(`, `check_proxy_routed(`, `check_proxy_url(` and `check_proxy_url_via_forwarder(`
in `src/` outside the three client files and the tests, and fails unless the list is exactly
the expected set: the three helpers, upstream's `check_proxy` (definition, body, test),
`rust_only::check_proxy_url` (no wallet, so no forwarder) and `check_proxy_url_via_forwarder`
(given one explicitly) with their definitions, and the TLS tests in `src/api/mod.rs`. A call
site passing `None` to a routed constructor, a direct constructor or check, or a second copy of
a helper's line fails it (checked by mutation). It is a grep: a client built under another name
escapes it, the same limit as the HTTP client guard.

`cargo test --locked --lib --features esplora,vss -- api::forwarder:: wallet::test::forwarder::`
(46 tests and a child test, local mockito servers, no regtest). The forwarder in the wallet
tests has a path, and every request must arrive on it. The tests of both modules share one
`serial_test` key and run one at a time: each holds several mockito servers, whose pool is 20 on
macOS, and run side by side they deadlocked waiting for one more (`29a49bb`).

- Client level (`api::forwarder::tests`, 19 and the child): every proxy method arrives with the
  right target and kind and nothing reaches the proxy, the reject list the same,
  `check_proxy_url_via_forwarder`; the target keeps scheme, port, path and query, and is
  `url::Url`'s serialization; userinfo or a fragment refused on every method before any
  request; a refusal is `ForwarderRefused` on every method, the check and the reject list,
  while a 403 without the header, the header on another status, or a missing or different
  session echo is not; the reason is cut by characters; a forwarded reject list fails closed on
  403, 503, 404, 500, 307, 203, 204 and 206, and on a 200 that holds text and no opout, while a
  direct one is read as upstream reads it; request errors name the target, not the forwarder;
  the forwarder's client ignores `HTTP_PROXY` (in a child process started with it set, after
  showing that an ordinary client does go there); without a forwarder the request goes direct
  with no `X-Era-*` header; a forwarder that is down, answers 403 or redirects: an error, and
  the proxy is never contacted; URL validation.
- Wallet level (`wallet::test::forwarder`, 27), each request sent from the code that sends it
  in production and required at the forwarder with both headers, never at the real host:
  `server.info` through `send_begin`; `consignment.get`, a NACK and `ack.get` through
  `refresh`; `consignment.post` and `media.post` through `post_transfer_data` (`send_end`
  needs a PSBT that spends real allocations); the ACK through `ack_consignment`, `media.get`
  through `fetch_and_save_attachments` and the reject list through `get_reject_list` (all three
  follow a consignment validated against a chain). Then: the reject list fails closed (and an
  empty 200 is an empty list); the refusal in `send_begin` (alone, the first of two, next to a
  usable endpoint), in `send_end` (alone, and before a usable endpoint that then takes the
  post), in the ACK poll, and in `consignment.get`, where it leaves the receive waiting and
  failable; a refusal reason never read as the proxy's "Cannot change ACK"; a refused or
  unroutable send failed by `fail_transfers`, an outage still keeping a send from failing, a
  blocked receive skipped by the bulk call; a forwarder that is down keeps today's errors in
  `send_begin` and `send_end`; userinfo or a fragment in an invoice endpoint fails
  `send_begin`, next to a usable one too; `go_online` switching the forwarder on and off,
  refusing bad URLs without changing the route, going offline when the probe of a new indexer
  fails (with the forwarder kept or dropped by the call; indexer and proxy traffic both checked)
  and not without a forwarder; `go_offline`; the invoice and the stored transfer keep the real
  proxy; without a forwarder the proxy is contacted directly.

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
list, an empty 200 refused, allow lines not counted): each fails at least one test. Scrubbing
the URL from body and decoding errors has no effect to test: reqwest 0.13 puts no URL there.

## 5. VSS restore checks (`45c3b39`, `777fe57`)

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

### What changes

- `restore_from_vss` (upstream's API, same signature) reads the manifest once: the download
  decrypts by the same answer the rename follows (`VssBackupClient::download_backup_with`). It
  reads the fingerprint before the download and uses it as a directory name only if it is one:
  8 hex characters, otherwise `Error::VssError` before any of the backup is fetched or written.
  Its log names neither the server URL nor the store ID (`45c3b39`).
- **`restore_from_vss_expecting(config, target_dir, expected_fingerprint)`** (fork-only,
  `777fe57`) is what the app calls. `expected_fingerprint` is the wallet's master fingerprint
  as rgb-lib names its directory, 8 lowercase hex characters; anything else is
  `Error::InvalidFingerprint` before anything is requested or written. Then, before any of the
  backup reaches the disk:
  - the wallet the server names must be this one, else `Error::FingerprintMismatch`;
  - with encryption enabled in the config (the default), a backup the manifest marks as
    unencrypted is **`Error::VssBackupUnencrypted`** (new variant, in the UDL): decrypting is
    what authenticates a backup;
  - an encrypted backup names its wallet inside, where the server cannot change it: that must
    be this one too, else `Error::FingerprintMismatch`.

  Each answer is read once, so the server cannot pass a check with one answer and have the
  restore act on another; the bridge's own reads before the call are no longer needed. The
  directory is `<target_dir>/<expected_fingerprint>`, as before.
- Still upstream's: with encryption disabled in the config, a plaintext backup is extracted into
  the target directory as it comes (entry names kept inside it by `zip`'s `enclosed_name`, but
  not limited to the `wallet/` directory). The app does not disable encryption.

### Tests

`cargo test --locked --lib --features esplora,vss -- wallet::vss::tests::` (24 tests, offline;
in `era.yml` since `45c3b39`). A scripted VSS server answers `getObject` per key and per read,
and counts the reads, so the host's read can be told the truth and rgb-lib's something else:
another wallet (`deadbeef`, `../escape`) on the second fingerprint read, a plaintext backup on
the second manifest read, an encrypted backup of another wallet; each refused, with nothing of
the backup fetched or written, in the target or next to it. Also: an expected fingerprint that
is not one refused before any request; the expected wallet restored with one read of each key;
upstream's `restore_from_vss` refusing a server fingerprint that is not one and reading the
manifest once; the restore log without the URL or store ID. Each check was removed once to see
its test fail (7 mutations, plus the two log lines put back).

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
  changed to skip `Indexer` and `Network` errors: keep theirs and add `ForwarderRefused` and
  `InvalidForwardTarget` to the errors it skips (`a14cdbb`). `vss.rs` merges cleanly on both.
  In `parse_recipient`, port the
  probe loop of the fork's `send_begin_impl`: the `refused` variable,
  `self.check_proxy_endpoint(..)` and the match on its result (`InvalidForwardTarget` returns
  at once, the first `ForwarderRefused` is kept), and the check of `refused` before
  `InvalidTransportEndpoints`. The proxy forwarder guard fails while `parse_recipient` still
  calls `check_proxy` (on both tags that is the one extra line); a probe without the refusal
  and target handling only the forwarder tests catch
  (`send_begin_reports_the_forwarders_refusal`,
  `send_begin_refuses_an_endpoint_with_userinfo_or_a_fragment`).
- If the proxy forwarder guard fires, route the new call through
  `WalletOnline::proxy_client`, `reject_list_client` or `check_proxy_endpoint`, and change the
  expected set in `era.yml` only for a line that is not a call site; do not widen the
  exclusion.
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
