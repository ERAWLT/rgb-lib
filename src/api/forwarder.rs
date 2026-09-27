//! ERA fork: a loopback forwarder for rgb-lib's RGB proxy and reject-list traffic.
//!
//! A host that routes every network request through its own code (the ERA app decides in a
//! forwarder it runs on loopback where each request may go) can point rgb-lib's indexer at that
//! forwarder, but not the RGB proxy: the proxy endpoint is a property of each invoice, public by
//! necessity, and a reject-list URL is a property of an asset contract. With a [`Forwarder`] set
//! through [`OnlineOptions::forwarder_url`], [`ProxyClient`] and [`RejectListClient`] send every
//! request to the forwarder instead of its real URL, otherwise exactly as they would have sent
//! it, and name the real URL in [`FORWARD_TARGET_HEADER`]. A real URL with userinfo or a fragment
//! is not sent at all ([`Error::InvalidForwardTarget`]). Invoices, stored transport endpoints and
//! everything else rgb-lib shows keep the real URL. `ERA.md` spells the contract out.
//!
//! A forwarder that will not carry a request answers 403 with [`FORWARD_REFUSED_HEADER`] (and, when
//! its URL has a path, [`FORWARD_SESSION_HEADER`] echoing it), and the clients report
//! [`Error::ForwarderRefused`]; any other answer is read as the target's.
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
/// Header of a forwarder's refusal, sent with status 403: its value is the reason.
pub(crate) const FORWARD_REFUSED_HEADER: &str = "x-era-forward-refused";
/// Header of a forwarder's refusal that names the path of the forwarder's URL, echoing it back.
pub(crate) const FORWARD_SESSION_HEADER: &str = "x-era-forward-session";
/// The longest refusal reason kept, in characters.
const MAX_REFUSAL_REASON: usize = 256;

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
    /// not `http`/`https`, no host) is refused here as `unsupported` (the caller's own error
    /// variant), so it fails the way it fails without a forwarder.
    ///
    /// A target with userinfo or a fragment is [`Error::InvalidForwardTarget`], and no request is
    /// made. The forwarder checks the target against an allowlist, and neither part has a place
    /// in an RGB proxy or reject-list URL; userinfo can put a trusted-looking name before the
    /// real host (`https://known.host@other.host/`), which a forwarder matching text instead of
    /// the parsed host would take for the known one.
    pub(crate) fn request(
        &self,
        client: &RestClient,
        method: reqwest::Method,
        target: &str,
        kind: &'static str,
        unsupported: impl FnOnce(String) -> Error,
    ) -> Result<reqwest::blocking::RequestBuilder, Error> {
        let target = match Url::parse(target) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => url,
            Ok(url) => return Err(unsupported(format!("unsupported target URL: {url}"))),
            Err(e) => return Err(unsupported(format!("invalid target URL: {e}"))),
        };
        if !target.username().is_empty() || target.password().is_some() {
            return Err(Error::InvalidForwardTarget {
                details: format!(
                    "a forwarded URL may not carry userinfo (its host is {})",
                    target.host_str().unwrap_or_default()
                ),
            });
        }
        if target.fragment().is_some() {
            return Err(Error::InvalidForwardTarget {
                details: format!("a forwarded URL may not carry a fragment: {target}"),
            });
        }
        Ok(client
            .request(method, self.url.clone())
            .header(FORWARD_TARGET_HEADER, target.as_str())
            .header(FORWARD_KIND_HEADER, kind))
    }

    /// `e`, from a request to the forwarder meant for `target`, naming `target` instead of the
    /// forwarder. reqwest puts the URL it requested in the text of an error, rgb-lib returns that
    /// text and logs some of it, and the forwarder's path may be a per-session secret.
    pub(crate) fn scrub(e: reqwest::Error, target: &str) -> reqwest::Error {
        if e.url().is_none() {
            return e;
        }
        match Url::parse(target) {
            Ok(target) => e.with_url(target),
            Err(_) => e.without_url(),
        }
    }

    /// The forwarder's refusal of a request meant for `target`, if `response` is one: status 403
    /// with [`FORWARD_REFUSED_HEADER`], whose value (possibly empty) is the reason, and, when the
    /// forwarder's URL has a path, [`FORWARD_SESSION_HEADER`] naming that path exactly. Anything
    /// else is the target's answer or the forwarder failing, and the caller reads it as it would
    /// read the target's own answer.
    ///
    /// A target's answer relayed with its headers could carry a refusal header of its own
    /// making; it cannot carry the path, which holds the session's secret and never leaves the
    /// device. A forwarder at the root has no path to echo, and its refusals go unchecked.
    pub(crate) fn refusal(
        &self,
        response: &reqwest::blocking::Response,
        target: &str,
    ) -> Option<Error> {
        if response.status() != reqwest::StatusCode::FORBIDDEN {
            return None;
        }
        let reason = response.headers().get(FORWARD_REFUSED_HEADER)?;
        let path = self.url.path();
        if path != "/"
            && response
                .headers()
                .get(FORWARD_SESSION_HEADER)
                .map(|echo| echo.as_bytes())
                != Some(path.as_bytes())
        {
            return None;
        }
        Some(Error::ForwarderRefused {
            // as the target header named it
            target: Url::parse(target).map_or_else(|_| target.to_string(), String::from),
            reason: refusal_reason(reason),
        })
    }
}

/// The reason of a refusal: the header's value, decoded lossily (its bytes need not be UTF-8) and
/// cut at [`MAX_REFUSAL_REASON`] characters.
fn refusal_reason(value: &reqwest::header::HeaderValue) -> String {
    String::from_utf8_lossy(value.as_bytes())
        .chars()
        .take(MAX_REFUSAL_REASON)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use mockito::{Matcher, Server, ServerGuard};
    use serde_json::json;
    // one at a time, with wallet::test::forwarder: see there
    use serial_test::serial;

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
    #[serial(forwarder)]
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
    #[serial(forwarder)]
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
    #[serial(forwarder)]
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
    #[serial(forwarder)]
    fn the_target_header_is_the_parsed_url() {
        // what the forwarder compares against its allowlist: url::Url's serialization
        let mut fwd = Server::new();
        for (given, sent) in [
            // host lower-cased and in punycode, the scheme's default port dropped
            (
                "https://PROXY.Exämple.COM:443/json-rpc",
                "https://proxy.xn--exmple-cua.com/json-rpc",
            ),
            // an IPv4 address in another notation
            (
                "http://0x7f.1:3000/json-rpc",
                "http://127.0.0.1:3000/json-rpc",
            ),
            // dot segments resolved, a space percent-encoded
            (
                "https://proxy.example.com/a/../json rpc",
                "https://proxy.example.com/json%20rpc",
            ),
            // an empty userinfo is no userinfo
            (
                "https://@proxy.example.com/json-rpc",
                "https://proxy.example.com/json-rpc",
            ),
        ] {
            let mock = forwarded_json(&mut fwd, sent, "ack.get", json!(false));
            let client = ProxyClient::new_routed(given, Some(&forwarder(&fwd))).unwrap();
            assert_eq!(
                client.get_ack("rid").unwrap().result,
                Some(false),
                "{given}"
            );
            mock.assert();
        }
    }

    #[test]
    #[serial(forwarder)]
    fn a_target_with_userinfo_or_a_fragment_is_refused_before_any_request() {
        let mut fwd = Server::new();
        let never = untouchable(&mut fwd);
        let file = tempfile::NamedTempFile::new().unwrap();
        for target in [
            // what rgb-lib derives from rpcs://rgb-proxy.utexo.com@evil.example/json-rpc: the
            // host is evil.example
            "https://rgb-proxy.utexo.com@evil.example/json-rpc",
            "http://user:password@proxy.example.com/json-rpc",
            "http://:password@proxy.example.com/json-rpc",
            "https://proxy.example.com/json-rpc#fragment",
            "https://proxy.example.com/json-rpc#",
        ] {
            let refused = |result: Result<(), Error>| {
                assert!(
                    matches!(result, Err(Error::InvalidForwardTarget { .. })),
                    "{target}: {result:?}"
                )
            };
            let client = ProxyClient::new_routed(target, Some(&forwarder(&fwd))).unwrap();
            refused(client.get_info().map(|_| ()));
            refused(client.get_ack("rid").map(|_| ()));
            refused(client.get_consignment("rid").map(|_| ()));
            refused(client.get_media("digest").map(|_| ()));
            refused(client.post_ack("rid", true).map(|_| ()));
            refused(
                client
                    .post_consignment("rid", file.path(), "00", Some(1))
                    .map(|_| ()),
            );
            refused(client.post_media("digest", file.path()).map(|_| ()));
            refused(crate::utils::check_proxy_routed(
                target,
                Some(&forwarder(&fwd)),
            ));
            refused(crate::wallet::rust_only::check_proxy_url_via_forwarder(
                target,
                &fwd.url(),
            ));
            let reject = RejectListClient::new_routed(target, Some(&forwarder(&fwd))).unwrap();
            refused(reject.get_reject_list().map(|_| ()));
        }
        never.assert();

        // without a forwarder nothing changes: the request is made as upstream makes it (here to
        // a port nothing listens on)
        let upstream = ProxyClient::new_routed("http://user@127.0.0.1:1/json-rpc", None).unwrap();
        assert_matches!(upstream.get_info(), Err(Error::Proxy { .. }));
    }

    #[test]
    #[serial(forwarder)]
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
    #[serial(forwarder)]
    fn reject_list_goes_through_the_forwarder() {
        let mut issuer = Server::new();
        let mut fwd = Server::new();
        let target = format!("{}/lists/usdt.txt", issuer.url());
        let direct = untouchable(&mut issuer);
        let body = format!(
            "{}\n",
            Opout::new(rgbstd::OpId::from([1u8; 32]), OS_ASSET, 0)
        );
        let list = fwd
            .mock("GET", "/")
            .match_header(FORWARD_TARGET_HEADER, target.as_str())
            .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST)
            .with_body(&body)
            .expect(1)
            .create();
        let client = RejectListClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        assert_eq!(client.get_reject_list().unwrap(), body);
        list.assert();
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn a_forwarded_reject_list_fails_closed() {
        let mut issuer = Server::new();
        let target = format!("{}/lists/usdt.txt", issuer.url());
        let direct = untouchable(&mut issuer);
        // read as a list, an error page holds no opout, which would validate the asset against
        // an empty list: whatever the body, only a 200 is a list, not even another 2xx
        for status in [403, 503, 404, 500, 307, 203, 204, 206] {
            let mut fwd = Server::new();
            let answer = fwd
                .mock("GET", "/")
                .match_header(FORWARD_TARGET_HEADER, target.as_str())
                .match_header(FORWARD_KIND_HEADER, FORWARD_KIND_REJECT_LIST)
                .with_status(status)
                .with_body(if status == 204 {
                    ""
                } else {
                    "<html>not a reject list</html>\n"
                })
                .expect(1)
                .create();
            let client = RejectListClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
            let result = client.get_reject_list();
            assert!(
                matches!(&result, Err(Error::RejectListService { details })
                    if details.contains(&status.to_string())),
                "{status}: {result:?}"
            );
            answer.assert();
        }
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn a_forwarded_200_is_a_list_only_if_empty_or_holding_an_opout() {
        let mut issuer = Server::new();
        let target = format!("{}/lists/usdt.txt", issuer.url());
        let direct = untouchable(&mut issuer);
        let opout = Opout::new(rgbstd::OpId::from([1u8; 32]), OS_ASSET, 0);
        for (body, list) in [
            (s!(""), true),
            (s!("\n"), true),
            (format!("{opout}\n"), true),
            (format!("not an opout\n!{opout}\n"), true),
            (s!("<html>captive portal</html>"), false),
            (s!(r#"{"error":"upstream timeout"}"#), false),
            (s!("line one\nline two\n"), false),
        ] {
            let mut fwd = Server::new();
            let answer = fwd.mock("GET", "/").with_body(&body).expect(1).create();
            let client = RejectListClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
            let result = client.get_reject_list();
            if list {
                assert_eq!(result.unwrap(), body);
            } else {
                assert!(
                    matches!(result, Err(Error::RejectListService { .. })),
                    "{body:?}: {result:?}"
                );
            }
            answer.assert();
        }
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn without_a_forwarder_the_reject_list_is_read_as_upstream_reads_it() {
        // upstream reads the body of any answer as the list; the fork changes only the routed
        // path, so this stays as it is (ERA.md, proxy forwarder)
        let mut issuer = Server::new();
        let list = issuer
            .mock("GET", "/list")
            .with_status(503)
            .with_body("unavailable")
            .expect(1)
            .create();
        let client = RejectListClient::new_routed(&format!("{}/list", issuer.url()), None).unwrap();
        assert_eq!(client.get_reject_list().unwrap(), "unavailable");
        list.assert();
    }

    #[test]
    #[serial(forwarder)]
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
    #[serial(forwarder)]
    fn request_errors_name_the_target_not_the_forwarder() {
        // the forwarder's path may be a per-session secret, and rgb-lib returns the text of a
        // request error and logs some of it
        let target = "https://proxy.example.com/json-rpc";
        let details = |result: Result<(), Error>| match result {
            Err(Error::Proxy { details } | Error::RejectListService { details }) => details,
            other => panic!("{other:?}"),
        };

        // nothing listening
        let closed = Forwarder::new("http://127.0.0.1:1/session-secret/rgb").unwrap();
        let client = ProxyClient::new_routed(target, Some(&closed)).unwrap();
        let reject = RejectListClient::new_routed(target, Some(&closed)).unwrap();
        for text in [
            details(client.get_info().map(|_| ())),
            details(reject.get_reject_list().map(|_| ())),
        ] {
            assert!(!text.contains("session-secret"), "{text}");
            assert!(text.contains(target), "{text}");
        }

        // an answer that is not what the client reads (reqwest 0.13 puts no URL in a body or
        // decoding error, but that is its choice to change)
        let mut fwd = Server::new();
        let _answers = ["POST", "GET"].map(|method| {
            fwd.mock(method, "/session-secret/rgb")
                .with_header("content-length", "100")
                .with_body("not JSON-RPC, and shorter than announced")
                .create()
        });
        let secret = Forwarder::new(&format!("{}/session-secret/rgb", fwd.url())).unwrap();
        let client = ProxyClient::new_routed(target, Some(&secret)).unwrap();
        let reject = RejectListClient::new_routed(target, Some(&secret)).unwrap();
        for text in [
            details(client.get_info().map(|_| ())),
            details(reject.get_reject_list().map(|_| ())),
        ] {
            assert!(!text.contains("session-secret"), "{text}");
        }
    }

    #[test]
    #[serial(forwarder)]
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

        // a 403 without the refusal header is not a refusal: an error like a proxy that is down
        let mut fwd = Server::new();
        let forbidden = fwd.mock("POST", "/").with_status(403).expect(1).create();
        let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        assert_matches!(client.get_consignment("rid"), Err(Error::Proxy { .. }));
        forbidden.assert();

        direct.assert();
    }

    /// A forwarder answering every request with `status` and, if given, the refusal header.
    fn answering(status: usize, refusal: Option<&str>) -> (ServerGuard, [mockito::Mock; 2]) {
        let mut fwd = Server::new();
        let mocks = ["POST", "GET"].map(|method| {
            let mock = fwd.mock(method, Matcher::Any).with_status(status);
            match refusal {
                Some(reason) => mock.with_header(FORWARD_REFUSED_HEADER, reason),
                None => mock,
            }
            .with_body("refused by policy")
            .create()
        });
        (fwd, mocks)
    }

    #[test]
    #[serial(forwarder)]
    fn a_refusal_is_forwarder_refused_on_every_request() {
        let mut proxy = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);
        let (fwd, _mocks) = answering(403, Some("not-allowlisted"));
        let refused = |result: Result<(), Error>| {
            assert_eq!(
                result.unwrap_err(),
                Error::ForwarderRefused {
                    target: target.clone(),
                    reason: s!("not-allowlisted"),
                }
            )
        };

        let file = tempfile::NamedTempFile::new().unwrap();
        let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
        refused(client.get_info().map(|_| ()));
        refused(client.get_ack("rid").map(|_| ()));
        refused(client.get_consignment("rid").map(|_| ()));
        refused(client.get_media("digest").map(|_| ()));
        refused(client.post_ack("rid", true).map(|_| ()));
        refused(
            client
                .post_consignment("rid", file.path(), "00", Some(1))
                .map(|_| ()),
        );
        refused(client.post_media("digest", file.path()).map(|_| ()));
        // the proxy check reports it instead of "unable to connect to proxy"
        refused(crate::utils::check_proxy_routed(
            &target,
            Some(&forwarder(&fwd)),
        ));
        refused(crate::wallet::rust_only::check_proxy_url_via_forwarder(
            &target,
            &fwd.url(),
        ));
        direct.assert();

        let mut issuer = Server::new();
        let list_url = format!("{}/list.txt", issuer.url());
        let direct = untouchable(&mut issuer);
        let client = RejectListClient::new_routed(&list_url, Some(&forwarder(&fwd))).unwrap();
        assert_eq!(
            client.get_reject_list().unwrap_err(),
            Error::ForwarderRefused {
                target: list_url.clone(),
                reason: s!("not-allowlisted"),
            }
        );
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn a_refusal_counts_only_with_the_forwarders_path_echoed() {
        // a forwarder with a secret in its path: its refusal echoes the path, which a target
        // whose answer the forwarder relays with its headers does not know
        let mut proxy = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);
        let path = "/session-secret/rgb";
        for (echo, refused) in [
            (Some(path), true),
            (None, false),
            (Some("/another-session/rgb"), false),
            (Some("/session-secret/rgb/"), false),
        ] {
            let mut fwd = Server::new();
            let _answers = ["POST", "GET"].map(|method| {
                let answer = fwd
                    .mock(method, path)
                    .with_status(403)
                    .with_header(FORWARD_REFUSED_HEADER, "not-allowlisted");
                match echo {
                    Some(echo) => answer.with_header(FORWARD_SESSION_HEADER, echo),
                    None => answer,
                }
                .create()
            });
            let secret = Forwarder::new(&format!("{}{path}", fwd.url())).unwrap();
            let client = ProxyClient::new_routed(&target, Some(&secret)).unwrap();
            let reject = RejectListClient::new_routed(&target, Some(&secret)).unwrap();
            if refused {
                assert_matches!(client.get_ack("rid"), Err(Error::ForwarderRefused { .. }));
                assert_matches!(
                    reject.get_reject_list(),
                    Err(Error::ForwarderRefused { .. })
                );
            } else {
                // an ordinary 403: what a proxy that does not answer JSON-RPC and a list that
                // is not there come to
                let ack = client.get_ack("rid");
                assert!(matches!(ack, Err(Error::Proxy { .. })), "{echo:?}: {ack:?}");
                let list = reject.get_reject_list();
                assert!(
                    matches!(list, Err(Error::RejectListService { .. })),
                    "{echo:?}: {list:?}"
                );
            }
        }
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn a_refusal_reason_is_cut_by_characters() {
        // mockito sends only ASCII header values, so the decoding is checked on its own
        let long = "é".repeat(MAX_REFUSAL_REASON + 50);
        let reason =
            refusal_reason(&reqwest::header::HeaderValue::from_bytes(long.as_bytes()).unwrap());
        assert_eq!(reason, "é".repeat(MAX_REFUSAL_REASON));
        // bytes that are not UTF-8 are replaced, not dropped
        let value = reqwest::header::HeaderValue::from_bytes(b"no\xffpe").unwrap();
        assert_eq!(refusal_reason(&value), "no\u{fffd}pe");
    }

    const SYSTEM_PROXY_CHILD: &str = "RGB_LIB_ERA_SYSTEM_PROXY_CHILD";

    #[test]
    #[serial(forwarder)]
    fn the_forwarder_client_ignores_the_system_proxy() {
        // reqwest reads HTTP_PROXY when a client is built, and the environment is shared by every
        // test of this process: the part with HTTP_PROXY set runs alone, in a child process
        let mut system_proxy = Server::new();
        // what the child's own check sends through the system proxy, and nothing else
        let checked = system_proxy
            .mock("POST", Matcher::Any)
            .match_header("host", "proxy.invalid")
            .with_body(RPC_OK)
            .expect(1)
            .create();
        let never = untouchable(&mut system_proxy);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "api::forwarder::tests::the_forwarder_client_ignores_the_system_proxy_in_a_child",
                "--include-ignored",
                "--test-threads=1",
            ])
            .env(SYSTEM_PROXY_CHILD, "1")
            .env("HTTP_PROXY", system_proxy.url())
            .env("http_proxy", system_proxy.url())
            .env("ALL_PROXY", system_proxy.url())
            .env("all_proxy", system_proxy.url())
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .env_remove("REQUEST_METHOD")
            .output()
            .unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.status.success(), "{log}");
        assert!(log.contains("test result: ok. 1 passed"), "{log}");
        checked.assert();
        never.assert();
    }

    #[test]
    #[ignore = "run by the_forwarder_client_ignores_the_system_proxy, with HTTP_PROXY set"]
    fn the_forwarder_client_ignores_the_system_proxy_in_a_child() {
        if std::env::var_os(SYSTEM_PROXY_CHILD).is_none() {
            return;
        }
        let target = "http://proxy.invalid/json-rpc";
        // the environment does send a client to HTTP_PROXY: proxy.invalid resolves nowhere, so
        // an answer can only come from there
        let upstream = ProxyClient::new(target).unwrap();
        assert_eq!(upstream.get_ack("rid").unwrap().result, Some(true));
        // the forwarder's client does not go there
        let mut fwd = Server::new();
        let mock = forwarded_json(&mut fwd, target, "ack.get", json!(true));
        let client = ProxyClient::new_routed(target, Some(&forwarder(&fwd))).unwrap();
        assert_eq!(client.get_ack("rid").unwrap().result, Some(true));
        mock.assert();
    }

    #[test]
    #[serial(forwarder)]
    fn only_a_403_with_the_refusal_header_is_a_refusal() {
        let mut proxy = Server::new();
        let target = format!("{}/json-rpc", proxy.url());
        let direct = untouchable(&mut proxy);
        // a 403 without the header, the header on another status: the errors of today, those of
        // a proxy that does not answer JSON-RPC
        for (status, refusal) in [(403, None), (503, None), (503, Some("not-allowlisted"))] {
            let (fwd, _mocks) = answering(status, refusal);
            let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
            assert_matches!(client.get_ack("rid"), Err(Error::Proxy { .. }));
            assert_matches!(
                crate::utils::check_proxy_routed(&target, Some(&forwarder(&fwd))),
                Err(Error::Proxy { details }) if details == "unable to connect to proxy"
            );
            let reject = RejectListClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
            assert_matches!(
                reject.get_reject_list(),
                Err(Error::RejectListService { .. })
            );
        }
        // an empty reason is still a refusal; a long one is cut
        for (reason, kept) in [(s!(""), 0), ("r".repeat(1000), MAX_REFUSAL_REASON)] {
            let (fwd, _mocks) = answering(403, Some(&reason));
            let client = ProxyClient::new_routed(&target, Some(&forwarder(&fwd))).unwrap();
            assert_matches!(
                client.get_ack("rid"),
                Err(Error::ForwarderRefused { reason, .. }) if reason.len() == kept
            );
        }
        direct.assert();
    }

    #[test]
    #[serial(forwarder)]
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
