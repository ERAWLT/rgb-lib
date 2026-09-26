pub(crate) mod multisig_hub;
pub(crate) mod proxy;
pub(crate) mod reject_list;

use super::*;

const JSON: &str = "application/json";
const OCTET_STREAM: &str = "application/octet-stream";
const CONNECT_TIMEOUT: u64 = 10;
const READ_WRITE_TIMEOUT: u64 = 120;

/// Builder for every blocking HTTP client rgb-lib creates (RGB proxy, reject list, multisig hub,
/// DFNS).
///
/// TLS is rustls on the ring provider, verified against the bundled Mozilla root store
/// (webpki-roots). That is the stack the Esplora client (minreq) and the VSS client (bitreq)
/// already use, so the library carries one crypto backend instead of three. reqwest's own rustls
/// setup would add aws-lc and verify through rustls-platform-verifier, which on Android needs the
/// host app to initialise it over JNI and fails the first handshake otherwise.
pub(crate) fn rest_client_builder() -> Result<reqwest::blocking::ClientBuilder, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
    };
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::RestClientBuild {
            details: e.to_string(),
        })?
        .with_root_certificates(roots)
        .with_no_client_auth();
    // what reqwest itself offers when it builds the config (http2 feature on)
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(RestClient::builder().tls_backend_preconfigured(tls))
}

#[cfg(test)]
mod tests_rest_client_tls {
    use super::*;

    #[test]
    fn rest_clients_build_with_the_preconfigured_tls() {
        // a TLS config reqwest does not recognise makes build() fail, so this proves the config
        // is the rustls version reqwest links and that no other provider is needed
        rest_client_builder().unwrap().build().unwrap();
        proxy::ProxyClient::new("https://proxy.example").unwrap();
        reject_list::RejectListClient::new("https://reject.example").unwrap();
        multisig_hub::MultisigHubClient::new("https://hub.example", "token").unwrap();
    }

    #[test]
    #[ignore = "needs network access"]
    fn rest_client_talks_to_a_public_https_proxy() {
        // UTEXO's signet RGB proxy, behind a public CA
        let proxy = proxy::ProxyClient::new("https://rgb-proxy.utexo.com/json-rpc").unwrap();
        let info = proxy.get_info().unwrap().result.unwrap();
        assert!(!info.protocol_version.is_empty());
    }
}
