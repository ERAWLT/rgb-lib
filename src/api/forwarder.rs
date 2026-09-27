//! ERA fork: a loopback forwarder for rgb-lib's RGB proxy and reject-list traffic.
//!
//! A host that routes every network request through its own code (the ERA app filters hosts and
//! pins TLS in a forwarder it runs on loopback) can point rgb-lib's indexer at that forwarder, but
//! not the RGB proxy: the proxy endpoint is a property of each invoice, public by necessity, and a
//! reject-list URL is a property of an asset contract. With a [`Forwarder`] set through
//! [`OnlineOptions::forwarder_url`], [`ProxyClient`] and [`RejectListClient`] send every request
//! to the forwarder instead of its real URL, otherwise exactly as they would have sent it, and
//! name the real URL in [`FORWARD_TARGET_HEADER`]. Invoices, stored transport endpoints and
//! everything else rgb-lib shows keep the real URL. `ERA.md` spells the contract out.
//!
//! Without a forwarder the clients behave as upstream: the same client configuration, the same
//! requests, no extra header.

use url::Host;

use super::*;

/// Header carrying the absolute `http`/`https` URL the request is meant for.
pub(crate) const FORWARD_TARGET_HEADER: &str = "x-era-forward-target";
/// Header carrying the kind of service the request is meant for.
pub(crate) const FORWARD_KIND_HEADER: &str = "x-era-forward-kind";
/// [`FORWARD_KIND_HEADER`] of an RGB proxy JSON-RPC request.
pub(crate) const FORWARD_KIND_RGB_PROXY: &str = "rgb-proxy";
/// [`FORWARD_KIND_HEADER`] of a reject-list request.
pub(crate) const FORWARD_KIND_REJECT_LIST: &str = "reject-list";

/// A validated forwarder URL: plain `http` to a loopback IP literal.
///
/// `pub` like [`ProxyClient`]: the `api` module is crate-private, so this is not public API, but
/// `WalletOnline` (a supertrait of a public trait) returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forwarder {
    url: Url,
}

impl Forwarder {
    /// Validate `forwarder_url`.
    ///
    /// The forwarder has to be on this device: requests to it carry consignments and ACKs in the
    /// clear, and TLS to the real host is its job. So the scheme is `http`, the host a loopback
    /// IP literal (`127.0.0.0/8` or `::1`; not `localhost`, whose resolution rgb-lib does not
    /// control), with no credentials, query or fragment. A path is allowed; requests are sent to
    /// the URL as given, nothing is appended to it.
    pub(crate) fn new(forwarder_url: &str) -> Result<Self, Error> {
        let invalid = |details: String| Error::InvalidForwarderUrl { details };
        let url = Url::parse(forwarder_url).map_err(|e| invalid(e.to_string()))?;
        if url.scheme() != "http" {
            return Err(invalid(format!(
                "scheme must be http, found '{}'",
                url.scheme()
            )));
        }
        let loopback = match url.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !loopback {
            return Err(invalid(s!(
                "host must be a loopback IP literal (127.0.0.0/8 or ::1)"
            )));
        }
        if url.port() == Some(0) {
            return Err(invalid(s!("port 0 is not a listening port")));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid(s!("credentials are not allowed")));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(invalid(s!("a query or a fragment is not allowed")));
        }
        Ok(Self { url })
    }

    /// The client for requests to this forwarder: rgb-lib's usual builder and timeouts, plus no
    /// system proxy (the request must reach loopback, not an HTTP proxy named by the environment)
    /// and no redirects (a redirect is the one way a response could send a request somewhere
    /// other than the forwarder).
    pub(crate) fn client(&self) -> Result<RestClient, Error> {
        Ok(rest_client_builder()?
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .timeout(Duration::from_secs(READ_WRITE_TIMEOUT))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }

    /// Start a request meant for `target`, sent to the forwarder with `target` and `kind` in
    /// headers.
    ///
    /// `target` is parsed as reqwest parses a request URL, and the header carries the parsed form,
    /// which is the URL reqwest would have requested. A target reqwest would refuse (unparsable,
    /// not `http`/`https`, no host) is refused here, with a message the caller wraps in its own
    /// error variant, so it fails the way it fails without a forwarder.
    pub(crate) fn request(
        &self,
        client: &RestClient,
        method: reqwest::Method,
        target: &str,
        kind: &'static str,
    ) -> Result<reqwest::blocking::RequestBuilder, String> {
        let target = Url::parse(target).map_err(|e| format!("invalid target URL: {e}"))?;
        if !matches!(target.scheme(), "http" | "https") || !target.has_host() {
            return Err(format!("unsupported target URL: {target}"));
        }
        Ok(client
            .request(method, self.url.clone())
            .header(FORWARD_TARGET_HEADER, target.as_str())
            .header(FORWARD_KIND_HEADER, kind))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use mockito::{Matcher, Server, ServerGuard};
    use serde_json::json;

    use super::*;

    const RPC_OK: &str = r#"{"jsonrpc":"2.0","id":null,"result":true,"error":null}"#;

    fn forwarder(server: &ServerGuard) -> Forwarder {
        Forwarder::new(&server.url()).unwrap()
    }

    /// Catch-all mocks for the two methods rgb-lib's clients use (POST for the proxy, GET for
    /// reject lists) on a server no request may reach.
    pub(crate) struct Untouchable([mockito::Mock; 2]);

    impl Untouchable {
        pub(crate) fn on(server: &mut ServerGuard) -> Self {
            Self(["POST", "GET"].map(|method| {
                server
                    .mock(method, Matcher::Any)
                    .with_status(500)
                    .expect(0)
                    .create()
            }))
        }

        pub(crate) fn assert(&self) {
            for mock in &self.0 {
                mock.assert();
            }
        }

        pub(crate) fn remove(&self) {
            for mock in &self.0 {
                mock.remove();
            }
        }
    }

    fn untouchable(server: &mut ServerGuard) -> Untouchable {
        Untouchable::on(server)
    }

    /// A forwarder route for one JSON-RPC method meant for `target`.
    fn forwarded_json(
        server: &mut ServerGuard,
        target: &str,
        method: &str,
        result: serde_json::Value,
    ) -> mockito::Mock {
        server
            .mock("POST", "/")
            .match_header(FORWARD_TARGET_HEADER, target)
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            .match_header("content-type", JSON)
            .match_body(Matcher::PartialJson(
                json!({"method": method, "jsonrpc": "2.0"}),
            ))
            .with_header("content-type", JSON)
            .with_body(json!({"jsonrpc": "2.0", "id": null, "result": result}).to_string())
            .expect(1)
            .create()
    }

    /// A forwarder route for one multipart method meant for `target`.
    fn forwarded_multipart(server: &mut ServerGuard, target: &str, method: &str) -> mockito::Mock {
        server
            .mock("POST", "/")
            .match_header(FORWARD_TARGET_HEADER, target)
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_RGB_PROXY)
            .match_header(
                "content-type",
                Matcher::Regex(s!("^multipart/form-data; boundary=")),
            )
            .match_body(Matcher::Regex(format!(
                "name=\"method\"\r\n\r\n{method}\r\n"
            )))
            .with_header("content-type", JSON)
            .with_body(RPC_OK)
            .expect(1)
            .create()
    }

    #[test]
    fn accepts_loopback_http_only() {
        for ok in [
            "http://127.0.0.1:8080",
            "http://127.0.0.1:8080/",
            "http://127.1.2.3:1/rgb/proxy",
            "http://[::1]:8080/rgb",
            "http://127.0.0.1",
        ] {
            assert!(Forwarder::new(ok).is_ok(), "{ok} should be accepted");
        }
        for bad in [
            "",
            "127.0.0.1:8080",
            "https://127.0.0.1:8080",
            "rpc://127.0.0.1:8080",
            "http://localhost:8080",
            "http://10.0.2.2:8080",
            "http://192.168.1.10:8080",
            "http://0.0.0.0:8080",
            "http://[::]:8080",
            "http://[::ffff:127.0.0.1]:8080",
            "http://rgb-proxy.utexo.com",
            "http://127.0.0.1.nip.io:8080",
            "http://127.0.0.1:0",
            "http://user:pass@127.0.0.1:8080",
            "http://127.0.0.1:8080/?target=x",
            "http://127.0.0.1:8080/#x",
        ] {
            let result = Forwarder::new(bad);
            assert!(
                matches!(result, Err(Error::InvalidForwarderUrl { .. })),
                "{bad} should be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn every_proxy_request_goes_to_the_forwarder_with_its_target() {
        let mut proxy = Server::new();
        let mut fwd = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);
        let mocks = [
            forwarded_json(
                &mut fwd,
                &target,
                "server.info",
                json!({"protocol_version": "0.2", "version": "0.2.1", "uptime": 1}),
            ),
            forwarded_json(&mut fwd, &target, "ack.get", json!(true)),
            forwarded_json(
                &mut fwd,
                &target,
                "consignment.get",
                json!({"consignment": "AA==", "txid": "00", "vout": null, "validated": null}),
            ),
            forwarded_json(&mut fwd, &target, "media.get", json!("AA==")),
            forwarded_json(&mut fwd, &target, "ack.post", json!(true)),
            forwarded_multipart(&mut fwd, &target, "consignment.post"),
            forwarded_multipart(&mut fwd, &target, "media.post"),
        ];

        let file = tempfile::NamedTempFile::new().unwrap();
        let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        let info = client.get_info().unwrap().result.unwrap();
        assert_eq!(info.protocol_version, "0.2");
        assert_eq!(client.get_ack("rid").unwrap().result, Some(true));
        assert_eq!(
            client.get_consignment("rid").unwrap().result.unwrap().txid,
            "00"
        );
        assert_eq!(client.get_media("digest").unwrap().result.unwrap(), "AA==");
        assert_eq!(client.post_ack("rid", true).unwrap().result, Some(true));
        let posted = client
            .post_consignment("rid", file.path(), "00", Some(1))
            .unwrap();
        assert_eq!(posted.result, Some(true));
        let posted = client.post_media("digest", file.path()).unwrap();
        assert_eq!(posted.result, Some(true));

        for mock in mocks {
            mock.assert();
        }
        direct.assert();
    }

    #[test]
    fn the_target_keeps_scheme_port_path_and_query() {
        let mut fwd = Server::new();
        // what rgb-lib derives from rpcs://proxy.example.com:8443/0.2/json-rpc?x=1 and from the
        // rpc:// (no TLS) form of the same endpoint
        for target in [
            "https://proxy.example.com:8443/0.2/json-rpc?x=1",
            "http://proxy.example.com/json-rpc",
        ] {
            let mock = forwarded_json(&mut fwd, target, "ack.get", json!(false));
            let client = ProxyClient::new_routed(target, Some(&forwarder(&fwd))).unwrap();
            assert_eq!(client.get_ack("rid").unwrap().result, Some(false));
            mock.assert();
        }
    }

    #[test]
    fn check_proxy_goes_through_the_forwarder() {
        let mut proxy = Server::new();
        let mut fwd = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);
        let info = forwarded_json(
            &mut fwd,
            &target,
            "server.info",
            // the protocol version rgb-lib requires (utils::PROXY_PROTOCOL_VERSION)
            json!({"protocol_version": "0.2", "version": "x", "uptime": 1}),
        );
        crate::wallet::rust_only::check_proxy_url_via_forwarder(&target, &fwd.url()).unwrap();
        info.assert();
        direct.assert();

        assert_matches!(
            crate::wallet::rust_only::check_proxy_url_via_forwarder(&target, "http://10.0.0.1:1"),
            Err(Error::InvalidForwarderUrl { .. })
        );
    }

    #[test]
    fn reject_list_goes_through_the_forwarder() {
        let mut issuer = Server::new();
        let mut fwd = Server::new();
        let target = format!("{}/lists/usdt.txt", issuer.url());
        let direct = untouchable(&mut issuer);
        let list = fwd
            .mock("GET", "/")
            .match_header(FORWARD_TARGET_HEADER, target.as_str())
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST)
            .with_body("opout-1\n")
            .expect(1)
            .create();
        let client = RejectListClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        assert_eq!(client.get_reject_list().unwrap(), "opout-1\n");
        list.assert();
        direct.assert();
    }

    #[test]
    fn without_a_forwarder_requests_go_direct_and_carry_no_header() {
        let mut proxy = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let ack = proxy
            .mock("POST", "/json-rpc")
            .match_header(FORWARD_TARGET_HEADER, Matcher::Missing)
            .match_header(FORWARD_KIND_HEADER, Matcher::Missing)
            .match_body(Matcher::PartialJson(json!({"method": "ack.get"})))
            .with_body(RPC_OK)
            .expect(2)
            .create();
        // new() is upstream's constructor; new_routed(.., None) must be the same client
        let upstream = ProxyClient::new(&target).unwrap();
        let routed = ProxyClient::new_routed(&target, None).unwrap();
        assert_eq!(upstream.get_ack("rid").unwrap().result, Some(true));
        assert_eq!(routed.get_ack("rid").unwrap().result, Some(true));
        ack.assert();

        let mut issuer = Server::new();
        let list = issuer
            .mock("GET", "/list")
            .match_header(FORWARD_TARGET_HEADER, Matcher::Missing)
            .with_body("x")
            .expect(1)
            .create();
        let client = RejectListClient::new_routed(&format!("{}/list", issuer.url()), None).unwrap();
        assert_eq!(client.get_reject_list().unwrap(), "x");
        list.assert();
    }

    #[test]
    fn a_failing_forwarder_is_an_error_not_a_direct_request() {
        let mut proxy = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);

        // nothing listening
        let closed = Forwarder::new("http://127.0.0.1:1").unwrap();
        let client = ProxyClient::new_routed(&target, Some(&closed)).unwrap();
        assert_matches!(client.get_info(), Err(Error::Proxy { .. }));
        assert_matches!(client.post_ack("rid", true), Err(Error::Proxy { .. }));
        assert_matches!(
            crate::utils::check_proxy_routed(&target, Some(&closed)),
            Err(Error::Proxy { .. })
        );
        let reject = RejectListClient::new_routed(&target, Some(&closed)).unwrap();
        assert_matches!(
            reject.get_reject_list(),
            Err(Error::RejectListService { .. })
        );

        // a forwarder answering with a redirect to the real proxy is not followed
        let mut fwd = Server::new();
        let redirect = fwd
            .mock("POST", "/")
            .with_status(307)
            .with_header("location", &target)
            .expect(1)
            .create();
        let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        assert_matches!(client.get_ack("rid"), Err(Error::Proxy { .. }));
        redirect.assert();

        // a forwarder refusing the target (allowlist) fails the call like a proxy that is down
        let mut fwd = Server::new();
        let refused = fwd.mock("POST", "/").with_status(403).expect(1).create();
        let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        assert_matches!(client.get_consignment("rid"), Err(Error::Proxy { .. }));
        refused.assert();

        direct.assert();
    }

    #[test]
    fn a_bad_target_fails_like_it_does_without_a_forwarder() {
        let mut fwd = Server::new();
        let never = untouchable(&mut fwd);
        for target in ["not a url", "ftp://proxy.example.com/json-rpc"] {
            let routed = ProxyClient::new_routed(target, Some(&forwarder(&fwd))).unwrap();
            assert_matches!(routed.get_info(), Err(Error::Proxy { .. }));
            let upstream = ProxyClient::new(target).unwrap();
            assert_matches!(upstream.get_info(), Err(Error::Proxy { .. }));
        }
        never.assert();
    }
}
