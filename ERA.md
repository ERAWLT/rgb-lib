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
  condition (see [§2](#2-ci-1d26404-extended-in-f808c7f-07e16e5-e7bdaa4-and-bef0c03)). This clone
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
| `cf0afa5` | Apply a new forwarder before go_online probes a new indexer | Fork-only |
| `9718cac` | Keep the forwarder's URL out of request errors | Fork-only |
| `0996972` | Describe the reviewed forwarder contract in its rustdoc | Fork-only |

Later commits that touch only this file are part of the series too. `3f0a555` to `0996972`
answer the review of `e7bdaa4` ([§4](#4-proxy-forwarder-e7bdaa4)) and go with it wherever the
series is carried.

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

## 2. CI (`1d26404`, extended in `f808c7f`, `07e16e5`, `e7bdaa4` and `bef0c03`)

`.github/workflows/era.yml` has two jobs.

- **check** runs on push / PR to `era/**` and on manual dispatch: rustfmt, `cargo check`
  with the app's feature set and with the upstream default on top, a dependency guard
  over the app's mobile targets, the HTTP client guard (below), the proxy forwarder
  guard ([§4](#4-proxy-forwarder-e7bdaa4)), the unit tests above plus the REST-client
  TLS test, the forwarder tests and offline wallet tests, and the migration crate.
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

Reworked after an independent review (2026-09-27, "pass with issues") in `fafe3bd` (reject
list fails closed), `bef0c03` (the route comes only from the wallet), `ccd23e9` (refusal
signal), `8a0255f` (no userinfo or fragment in a target), `cf0afa5` (`go_online` applies the
forwarder before its indexer probe), `9718cac` (errors never name the forwarder) and
`0996972` (rustdoc). This section describes the result.

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
(`reject_list_url`), and consignment validation in `refresh` and `send_begin` fetch it. Any
asset sent to the wallet can name a host rgb-lib will then contact. So the patch routes it too.

### What the app calls

```rust
// rgb_lib::wallet::OnlineOptions gains one field (`forwarderUrl` with the camel_case feature)
wallet.go_online(OnlineOptions {
    indexer_url,                        // the forwarder's Esplora route (T1.8)
    skip_consistency_check: false,
    vanilla_sync_lookback,
    // None = upstream behaviour; the path may carry a per-session secret
    forwarder_url: Some(format!("http://127.0.0.1:{port}/{session_secret}/rgb")),
})?;

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
  request, can. rgb-lib keeps `forwarder_url` in memory only (the wallet's `OnlineData`): it is
  not written to the wallet's files, invoices, the database or a backup, not logged, and a
  failed request's error names the URL the request was meant for, never the forwarder's
  (`9718cac`).
- **`go_online` applies a valid value on every call, before anything that can fail.** A
  refused `forwarder_url` changes nothing. Otherwise the new forwarder (or `None`, back to
  direct connections) is in force as soon as `forwarder_url` has been validated, before the
  probe of a new `indexer_url` and before the consistency check: a call that fails there
  still moves the proxy traffic, while the wallet stays online on its previous indexer, as
  upstream keeps it (`cf0afa5`). The forwarder gets a new port and secret on every unlock, and
  the previous port may belong to another app by then. A first `go_online` that fails leaves
  the wallet offline, with no proxy traffic at all.
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
  not used), and no error of any kind leads to a direct connection. Wallet code gets these
  clients only from the wallet's own route ([Guard and tests](#guard-and-tests)).
- **Headers added**, and no others:
  - `X-Era-Forward-Target`: the absolute URL rgb-lib would have requested, as `url::Url`
    serializes it: scheme and host lower-case, a non-ASCII host in punycode, an IPv4 address
    in dotted decimal, the scheme's default port dropped, dot segments resolved, a character
    a path may not hold (a space) percent-encoded. An `rpcs://host/path` transport endpoint
    becomes `https://host/path`, `rpc://` becomes `http://` (upstream's
    `TransportEndpoint::try_from`).
    Always `http` or `https` with a host (anything else fails inside rgb-lib, as it does
    without a forwarder). Never userinfo, never a fragment: a URL carrying either is
    `Error::InvalidForwardTarget` and is not requested at all (`8a0255f`), as
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
    is the forwarder refusing the request. Its value is the reason, passed through (up to 256
    characters) in `Error::ForwarderRefused { target, reason }`, `target` being the
    `X-Era-Forward-Target` value (`ccd23e9`). Both parts are required: a 403 without the
    header, or the header on another status, is an ordinary answer;
  - **the reject list fails closed**: only a 2xx answer is read as the list; any other
    status is `Error::RejectListService` with the status in `details` (`fafe3bd`). Read as a
    list, an error page holds no opout, and the asset would be validated against an empty
    list.

  For `rgb-proxy`, any other answer is read as JSON-RPC whatever its status, exactly as rgb-lib
  reads the proxy's own; a body that is not a JSON-RPC response is `Error::Proxy`.

### How the forwarder must match a target

The allowlist decision is the forwarder's, on a URL that comes from a counterparty's invoice or
an asset contract. The rule:

1. Parse `X-Era-Forward-Target` as a URL, with a WHATWG-conformant parser (the form above is
   `url::Url`'s, so parsing and serializing it again gives the same string). Refuse the
   request if it does not parse, is not absolute `http` or `https`, or carries userinfo or a
   fragment: rgb-lib never sends those, so such a header is not rgb-lib's.
2. Compare **scheme**, **host** and **effective port** (the explicit port, else 443 for
   `https` and 80 for `http`) exactly with the allowlist entry, on the parsed values: the host
   as serialized (lower-case, punycode, dotted-decimal IPv4, bracketed IPv6).
3. Never match on the header text: no prefix, suffix or substring test, no `startsWith`
   (`https://rgb-proxy.utexo.com@evil.example/` starts with a known host), no wildcard beyond
   what an entry says explicitly.
4. The **path** is per allowlist entry (for example exactly `/json-rpc` for a proxy, the
   list's own path for a reject list), compared on the parsed path.
5. The **query** is not part of the match (a probe carries `rid_nonce`); relay it as it is.
6. An entry holds for one **kind**: a proxy entry does not admit a `reject-list` request, nor
   the other way round.

### The forwarder's side

Not rgb-lib's code; what the fork's contract expects of it, consistent with the app's
decision D7 as amended by the project lead on 2026-09-27:

- **Check the caller**: the path of `forwarder_url` must carry the session's secret; any other
  path is refused.
- **Strip** both `X-Era-Forward-*` headers before relaying; the `Host` is the target's.
- **A target on the allowlist** (per kind) is relayed to the app's own **backend**, which
  relays it to the target (N2: pinning, the backend's own controls).
- **An unknown recipient proxy** (`rgb-proxy` only) may be reached **directly by the
  forwarder itself, only after the user explicitly approved a warning** naming that host:
  system TLS, redirects not followed, no pinning (there is nothing to pin for an arbitrary
  host).
- **A reject list is allowlist-only**: there is no user to ask.
- **Everything else is refused with the refusal signal**: `403` and
  `X-Era-Forward-Refused: <reason>`, the reason a short token of the app's choosing (for
  example `not-allowlisted`, `user-declined`, `consent-expired`).
- **Relay the target's status and body as they came.** Never answer with a 3xx. For a failure
  of its own (the backend is down, the target cannot be reached, a pending call is cut short
  on lock) answer a non-2xx status **without** the refusal header, for example 503: rgb-lib
  then behaves as for a proxy that is down, and the call can be retried.

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
| reject list `GET` | `refresh` (receiving an IFA asset), `send_begin` (sending one) | `ForwarderRefused` | `RejectListService` (a non-2xx answer included) |

- A refused endpoint next to a usable one is skipped like an unreachable one, so an invoice
  listing an unknown proxy beside a known one sends through the known one without an error.
- `consignment.get` is deliberately unchanged: upstream reads every failure there as "no
  consignment yet", and `fail_transfers` refreshes a transfer before failing it, so an error
  there would make an expired receive impossible to fail for as long as the forwarder refuses.
  The refusal is visible where it is made, in the forwarder.
- A failing reject list (refused or not) leaves the transfer waiting with that failure on
  every `refresh`; as upstream does when the list is unreachable, `fail_transfers` cannot fail
  it meanwhile (it refreshes first). That is the cost of failing closed: a reject list host
  the app serves must be on its allowlist.
- `ForwarderRefused` is the forwarder's policy speaking: the same request gets the same
  answer until something changes on the forwarder's side (its allowlist, the user's consent).
  An outage keeps today's errors, which a retry can clear.
- `InvalidForwardTarget` fails `send_begin` at once, even when another endpoint of the
  recipient is usable: an invoice carrying such an endpoint is malformed or hostile, not a
  proxy that happens to be down. A stored endpoint only gets there through a `send_begin`
  made without a forwarder; its requests are never sent, and the error takes the path of a
  failed request in the table.

### What does not change

- Invoices, the transport endpoints stored with transfers and in the DB, and every value
  rgb-lib returns keep the real endpoint
  (`wallet::test::forwarder::proxy_traffic_goes_through_the_forwarder` checks the invoice and
  the stored transfer).
- With `forwarder_url` unset: the same client (upstream's `ProxyClient::new` /
  `RejectListClient::new`), the same requests, no extra header, the same errors. Upstream
  reads a direct reject list's answer as the list whatever its status (a 503 page is an empty
  list); the fork leaves that path as it is (a test pins it). Worth reporting to UTEXO.
- Not routed by this patch, because the host already chooses those URLs: the indexer
  (`indexer_url`), VSS (its server URL), the multisig hub and DFNS (not used by the app).
  On the `-bfa` bases, `OnlineOptions::eth_rpc_url` (BFA validation reads an Ethereum
  RPC) is the same kind of host-chosen URL: point it at the forwarder too.

### Public API the host adapts to

- `OnlineOptions::forwarder_url: Option<String>` (`e7bdaa4`).
- `rust_only::check_proxy_url_via_forwarder(proxy_url, forwarder_url)` (`e7bdaa4`); it now
  returns `ForwarderRefused` and `InvalidForwardTarget` as such.
- `Error::InvalidForwarderUrl { details }` (`e7bdaa4`), `Error::ForwarderRefused { target,
  reason }` (`ccd23e9`), `Error::InvalidForwardTarget { details }` (`8a0255f`), all three
  mirrored in the uniffi UDL, whose build fails on a missing one (`cargo check --locked` in
  `bindings/uniffi`; not in `era.yml`).
- Changed error on a routed path: `send_begin` returns `ForwarderRefused` where it returned
  `InvalidTransportEndpoints` ("no valid transport endpoints") because of a refusal, and
  `InvalidForwardTarget` for an endpoint with userinfo or a fragment; `send_end` returns
  `ForwarderRefused` where it returned `NoValidTransportEndpoint` because of a refusal; a
  forwarded reject list answering a non-2xx status is `RejectListService` where it used to be
  read as an (empty) list.

### Where it is

`src/api/forwarder.rs` (validation of `forwarder_url`, the client, the target check and the
headers in `Forwarder::request`, `Forwarder::refusal`, `Forwarder::scrub`, the client-level
tests), `ProxyClient::post` / `ProxyClient::call` and `RejectListClient::get_forwarded` (the
one place each builds, sends and reads a routed request), `WalletOnline::forwarder` /
`proxy_client` / `reject_list_client` / `check_proxy_endpoint` and `go_online_impl` in
`src/wallet/online.rs`, `utils::check_proxy_routed`, `OnlineOptions::forwarder_url`,
`OnlineData::forwarder`, the three error variants, `rust_only::check_proxy_url_via_forwarder`.

### Guard and tests

**The proxy forwarder guard** (`era.yml`, "Proxy and reject-list clients only through the
wallet's route"): wallet code reaches an RGB proxy or a reject list only through
`WalletOnline::proxy_client`, `reject_list_client` and `check_proxy_endpoint`, which read the
forwarder `go_online` stored and take no route from their caller. The step lists every
`ProxyClient::new(` / `::new_routed(`, `RejectListClient::new(` / `::new_routed(`,
`check_proxy(` and `check_proxy_routed(` in `src/` outside the three client files and the
tests, and fails unless the list is exactly the expected set: the three helpers, upstream's
`check_proxy` (definition, body, test), `rust_only::check_proxy_url` (no wallet, so no
forwarder) and `check_proxy_url_via_forwarder` (given one explicitly), and the TLS tests in
`src/api/mod.rs`. A call site passing `None` to a routed constructor, a direct constructor, or
a second copy of a helper's line fails it (checked by mutation). It is a grep: a client built
under another name escapes it, the same limit as the HTTP client guard.

`cargo test --locked --lib --features esplora,vss -- api::forwarder:: wallet::test::forwarder::`
(34 tests, local mockito servers, no regtest). The forwarder in the wallet tests has a path,
and every request must arrive on it.

- Client level (`api::forwarder::tests`, 15): every proxy method arrives with the right
  target and kind and nothing reaches the proxy, the reject list the same,
  `check_proxy_url_via_forwarder`; the target keeps scheme, port, path and query, and is
  `url::Url`'s serialization; userinfo or a fragment refused on every method before any
  request; a refusal is `ForwarderRefused` on every method, the check and the reject list,
  while a 403 without the header or the header on another status is not; the reason's limit;
  a forwarded reject list fails closed on 403, 503, 404, 500 and 307, a direct one is read as
  upstream reads it; request errors name the target, not the forwarder; without a forwarder the
  request goes direct with no `X-Era-*` header; a forwarder that is down, answers 403 or
  redirects: an error, and the proxy is never contacted; URL validation.
- Wallet level (`wallet::test::forwarder`, 19), each request sent from the code that sends it
  in production and required at the forwarder with both headers, never at the real host:
  `server.info` through `send_begin`; `consignment.get`, a NACK and `ack.get` through
  `refresh`; `consignment.post` and `media.post` through `post_transfer_data` (`send_end`
  needs a PSBT that spends real allocations); the ACK through `ack_consignment`, `media.get`
  through `fetch_and_save_attachments` and the reject list through `get_reject_list` (all three
  follow a consignment validated against a chain). Then: the reject list fails closed; the
  refusal in `send_begin` (alone, and next to a usable endpoint), in `send_end`, in the ACK
  poll, and in `consignment.get`, where it leaves the receive waiting and failable; a
  forwarder that is down keeps today's errors in `send_begin` and `send_end`; userinfo or a
  fragment in an invoice endpoint fails `send_begin`, next to a usable one too; `go_online`
  switching the forwarder on and off, refusing bad URLs without changing the route, and moving
  it even when the probe of a new indexer fails; the invoice and the stored transfer keep the
  real proxy; without a forwarder the proxy is contacted directly.

Mutations checked on 2026-09-27, each failing at least one of these tests: any one call site
or helper going direct or passing `None` (11), the refusal ignored by the proxy client, the
reject-list client, `check_proxy`, `send_begin` or `send_end`, a refusal reported next to a
usable endpoint, a 403 taken as a refusal without the header, the header taken on another
status, the reason not cut (9), userinfo, a lone password or a fragment let through,
`check_proxy` or `send_begin` swallowing an invalid target (5), the forwarder applied only
after the indexer probe or not applied to new online data (2), a forwarded non-2xx reject list
read as a list, the forwarder's URL left in a send error. Scrubbing the URL from body and
decoding errors has no effect to test: reqwest 0.13 puts no URL there. Of the mutations of
`e7bdaa4`, those the new tests do not already repeat were run again (redirects followed, any
host accepted, a wrong target header, no kind header, the forwarder cleared before
validation): each still fails.

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
- The review fixes after it (`3f0a555` to `0996972`) were checked against both tags with
  `git merge-tree --merge-base=62a8c3a <tag> <this branch>`, the series as one diff
  (2026-09-27; not cherry-picked one by one, not compiled): they conflict nowhere `e7bdaa4`
  does not, except in `Error` and the UDL on `v0.3.0-beta.43-bfa`, where `ForwarderRefused`
  lands next to UTEXO's `PsbtOperationNotFound` (keep both). In `parse_recipient`, port the
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
