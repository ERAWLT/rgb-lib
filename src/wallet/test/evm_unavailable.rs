//! ERA fork (S2): an EVM chain whose endpoint fails its check is left out of the session instead of
//! failing `go_online` (`OnlineOptions::defer_unavailable_evm_chains`), and is checked again by a
//! refresh with an incoming transfer to process, by `check_unavailable_evm_chains` and by the next
//! `go_online`. Wallets on the scripted chain (every schema, BFA included), EVM endpoints served
//! by mockito or not at all. A received BFA consignment waiting for such a chain is tested on
//! regtest (`era_regtest`), where an EVM node and a bridge exist. See the fork's `ERA.md`.

use super::scripted_chain::*;
use super::*;

/// An endpoint nothing listens on, its path standing in for the forwarder's session secret.
const DOWN_URL: &str = "http://127.0.0.1:1/SECRET/evm";

fn options(chain: &ScriptedChain, rpc_url: &str, defer: bool) -> OnlineOptions {
    OnlineOptions {
        evm_chains: test_evm_chains(HashMap::from([(ANVIL_CHAIN_ID, rpc_url.to_string())])),
        defer_unavailable_evm_chains: defer,
        ..online_options(chain)
    }
}

fn rpc_mock(server: &mut mockito::Server, method: &str, body: &str) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(mockito::Matcher::PartialJsonString(format!(
            r#"{{"method":"{method}"}}"#
        )))
        .with_body(format!(r#"{{"jsonrpc":"2.0","id":1,{body}}}"#))
        .create()
}

/// An endpoint on `chain_id` serving what the check asks: the chain ID, a finalized block and the
/// state as of it.
fn serve(server: &mut mockito::Server, chain_id: u64) -> Vec<mockito::Mock> {
    vec![
        rpc_mock(
            server,
            "eth_chainId",
            &format!(r#""result":"0x{chain_id:x}""#),
        ),
        rpc_mock(
            server,
            "eth_getBlockByNumber",
            r#""result":{"number":"0x10"}"#,
        ),
        rpc_mock(server, "eth_call", r#""result":"0x""#),
    ]
}

/// An endpoint that answers every request with a 500 (not retried), `times` times exactly.
fn fail(server: &mut mockito::Server, times: usize) -> mockito::Mock {
    server
        .mock("POST", "/")
        .with_status(500)
        .expect(times)
        .create()
}

fn in_use(wallet: &Wallet) -> HashMap<u64, String> {
    wallet.online_data().as_ref().unwrap().evm_rpc_urls.clone()
}

fn left_out(wallet: &Wallet) -> Vec<u64> {
    wallet
        .unavailable_evm_chains()
        .into_iter()
        .map(|c| c.chain_id)
        .collect()
}

/// A funded wallet with colored UTXOs, offline (to go online with the options a test gives).
fn funded_offline(chain: &ScriptedChain) -> Wallet {
    let (mut wallet, _) = funded(chain, 2);
    wallet.go_offline();
    wallet
}

/// A blind receive on the chain's proxy: an incoming transfer waiting for its counterparty.
fn blind_receive(wallet: &mut Wallet, online: Online, chain: &ScriptedChain) -> ReceiveData {
    wallet
        .blind_receive(
            Some(online),
            None,
            Assignment::Any,
            default_rcv_expiration(),
            vec![chain.proxy_endpoint()],
            MIN_CONFIRMATIONS,
            None,
        )
        .unwrap()
}

/// Without the option, an endpoint that is down fails `go_online`, as upstream's does.
#[test]
#[parallel]
fn without_the_option_a_chain_down_fails_go_online() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let result = wallet.go_online(options(&chain, DOWN_URL, false));
    assert_matches!(result, Err(Error::InvalidEvmRpcUrl { .. }));
    assert!(wallet.online_data().is_none());
    assert!(wallet.unavailable_evm_chains().is_empty());
}

/// With it, the wallet goes online without the chain, which is reported with the error's text,
/// the endpoint named by its scheme and host only; the bitcoins and the other schemas work.
#[test]
#[parallel]
fn with_the_option_a_chain_down_is_left_out() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let online = wallet.go_online(options(&chain, DOWN_URL, true)).unwrap();
    assert!(in_use(&wallet).is_empty());
    assert!(wallet.online_data().as_ref().unwrap().evm_chains.is_empty());
    let unavailable = wallet.unavailable_evm_chains();
    assert_eq!(unavailable.len(), 1, "{unavailable:?}");
    assert_eq!(unavailable[0].chain_id, ANVIL_CHAIN_ID);
    assert!(
        unavailable[0].details.contains("EVM"),
        "{}",
        unavailable[0].details
    );
    assert!(
        !unavailable[0].details.contains("SECRET"),
        "{}",
        unavailable[0].details
    );

    // the rest of the wallet is online: an NIA asset, a refresh, a blind receive
    let asset_id = issue(&wallet, vec![AMOUNT]);
    blind_receive(&mut wallet, online, &chain);
    wallet.refresh(online, None, vec![], false).unwrap();
    assert_eq!(wallet.get_asset_balance(asset_id).unwrap().settled, AMOUNT);
    // and the chain is still left out (nothing listens there)
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);
}

/// An endpoint that is not an EVM JSON-RPC server, or one on another chain, is left out too.
#[test]
#[parallel]
fn with_the_option_a_wrong_endpoint_is_left_out() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);

    // a server that answers, but not JSON-RPC
    let mut server = mockito::Server::new();
    let page = server
        .mock("POST", "/")
        .with_header("content-type", "text/html")
        .with_body("<html>not a node</html>")
        .create();
    wallet
        .go_online(options(&chain, &server.url(), true))
        .unwrap();
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);
    assert!(in_use(&wallet).is_empty());
    wallet.go_offline();
    page.remove();

    // on another chain
    let _mocks = serve(&mut server, ANVIL_CHAIN_ID + 1);
    wallet
        .go_online(options(&chain, &server.url(), true))
        .unwrap();
    let unavailable = wallet.unavailable_evm_chains();
    assert_eq!(unavailable.len(), 1);
    assert_eq!(
        unavailable[0].details,
        Error::EvmChainIdMismatch {
            expected: ANVIL_CHAIN_ID,
            actual: ANVIL_CHAIN_ID + 1,
        }
        .to_string()
    );
    assert!(in_use(&wallet).is_empty());
}

/// The host's own mistake (a minimum mint lock above the maximum) fails `go_online` with the option
/// too, before any endpoint is asked anything.
#[test]
#[parallel]
fn inconsistent_mint_lock_blocks_fail_go_online_with_the_option() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let mut server = mockito::Server::new();
    let untouched = fail(&mut server, 0);
    let mut online_options = options(&chain, &server.url(), true);
    online_options.evm_chains = HashMap::from([(
        ANVIL_CHAIN_ID,
        EvmChain {
            rpc_url: server.url(),
            min_mint_lock_blocks: 11,
            max_mint_lock_blocks: 10,
        },
    )]);
    let result = wallet.go_online(online_options);
    assert_matches!(result, Err(Error::InvalidEvmChain { .. }));
    assert!(wallet.online_data().is_none());
    untouched.assert();
}

/// A refresh with an incoming transfer to process checks the chain again, and uses it once its
/// check passes; one with none asks the endpoint nothing.
#[test]
#[parallel]
fn a_refresh_with_an_incoming_transfer_uses_a_chain_back() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let mut server = mockito::Server::new();

    // going online asks once (a 500 is not sent again)
    let down = fail(&mut server, 1);
    let online = wallet
        .go_online(options(&chain, &server.url(), true))
        .unwrap();
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);
    // no transfer: a refresh asks nothing
    wallet.refresh(online, None, vec![], false).unwrap();
    down.assert();
    down.remove();

    // an incoming transfer: each refresh asks again, the chain still down
    blind_receive(&mut wallet, online, &chain);
    let down = fail(&mut server, 2);
    wallet.refresh(online, None, vec![], false).unwrap();
    wallet.refresh(online, None, vec![], false).unwrap();
    down.assert();
    down.remove();
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);
    assert!(in_use(&wallet).is_empty());

    // the chain is back: the next refresh uses it, and the one after asks no more
    let _mocks = serve(&mut server, ANVIL_CHAIN_ID);
    wallet.refresh(online, None, vec![], false).unwrap();
    assert!(wallet.unavailable_evm_chains().is_empty());
    assert_eq!(
        in_use(&wallet),
        HashMap::from([(ANVIL_CHAIN_ID, server.url())])
    );
    assert_eq!(
        wallet.online_data().as_ref().unwrap().evm_chains,
        test_evm_chains(HashMap::from([(ANVIL_CHAIN_ID, server.url())]))
    );
    let untouched = fail(&mut server, 0);
    wallet.refresh(online, None, vec![], false).unwrap();
    untouched.assert();
}

/// The host checks the chains left out itself before an operation that needs one.
#[test]
#[parallel]
fn check_unavailable_evm_chains_uses_a_chain_back() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let mut server = mockito::Server::new();
    let down = fail(&mut server, 2);
    let online = wallet
        .go_online(options(&chain, &server.url(), true))
        .unwrap();
    let still = wallet.check_unavailable_evm_chains(online).unwrap();
    assert_eq!(
        still.iter().map(|c| c.chain_id).collect::<Vec<_>>(),
        vec![ANVIL_CHAIN_ID]
    );
    down.assert();
    down.remove();

    let _mocks = serve(&mut server, ANVIL_CHAIN_ID);
    assert!(
        wallet
            .check_unavailable_evm_chains(online)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        in_use(&wallet),
        HashMap::from([(ANVIL_CHAIN_ID, server.url())])
    );
    // nothing left out: nothing asked
    let untouched = fail(&mut server, 0);
    assert!(
        wallet
            .check_unavailable_evm_chains(online)
            .unwrap()
            .is_empty()
    );
    untouched.assert();

    // offline: refused, as every online call
    wallet.go_offline();
    assert_matches!(
        wallet.check_unavailable_evm_chains(online),
        Err(Error::Offline)
    );
    assert!(wallet.unavailable_evm_chains().is_empty());
}

/// Going online again (the same indexer) replaces both sets: a chain back is used, and a chain in
/// use whose endpoint now fails is left out, its earlier URL no longer used. A wallet a forwarder
/// routes stays online then, where upstream's check takes it offline.
#[test]
#[parallel]
fn going_online_again_replaces_the_chains_left_out() {
    let chain = ScriptedChain::start();
    let mut wallet = funded_offline(&chain);
    let routed = |rpc_url: &str| OnlineOptions {
        forwarder_url: Some(chain.forwarder_url()),
        ..options(&chain, rpc_url, true)
    };

    let online = wallet.go_online(routed(DOWN_URL)).unwrap();
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);

    let mut server = mockito::Server::new();
    let mocks = serve(&mut server, ANVIL_CHAIN_ID);
    assert_eq!(wallet.go_online(routed(&server.url())).unwrap(), online);
    assert!(wallet.unavailable_evm_chains().is_empty());
    assert_eq!(
        in_use(&wallet),
        HashMap::from([(ANVIL_CHAIN_ID, server.url())])
    );

    for mock in mocks {
        mock.remove();
    }
    let _down = fail(&mut server, 1);
    assert_eq!(wallet.go_online(routed(&server.url())).unwrap(), online);
    assert_eq!(left_out(&wallet), vec![ANVIL_CHAIN_ID]);
    assert!(in_use(&wallet).is_empty());
    assert!(wallet.online_data().as_ref().unwrap().forwarder.is_some());

    // without the option, the same failure takes the routed wallet offline (the fork's ff6e73a)
    let result = wallet.go_online(OnlineOptions {
        defer_unavailable_evm_chains: false,
        ..routed(DOWN_URL)
    });
    assert_matches!(result, Err(Error::InvalidEvmRpcUrl { .. }));
    assert!(wallet.online_data().is_none());
    assert!(wallet.unavailable_evm_chains().is_empty());
}

/// A wallet that does not hold BFA checks no chain, with the option or without it.
#[test]
#[parallel]
fn a_wallet_without_bfa_checks_no_chain() {
    let chain = ScriptedChain::start();
    create_test_data_dir();
    let keys = generate_keys(BitcoinNetwork::Regtest, WitnessVersion::Taproot);
    let mut wallet = Wallet::new(
        WalletData {
            supported_schemas: vec![AssetSchema::Nia],
            ..get_test_wallet_data(&get_test_data_dir_string())
        },
        SinglesigKeys::from_keys(&keys, None),
    )
    .unwrap();
    let mut server = mockito::Server::new();
    let untouched = fail(&mut server, 0);
    for defer in [false, true] {
        wallet
            .go_online(options(&chain, &server.url(), defer))
            .unwrap();
        assert!(wallet.unavailable_evm_chains().is_empty());
        assert!(in_use(&wallet).is_empty());
        wallet.go_offline();
    }
    untouched.assert();
}
