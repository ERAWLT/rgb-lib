use super::{
    forwarder::{FORWARD_KIND_REJECT_LIST, Forwarder},
    *,
};

pub struct RejectListClient {
    client: RestClient,
    base_url: String,
    // ERA fork: when set, requests go to this loopback forwarder instead of base_url
    forwarder: Option<Forwarder>,
}

impl RejectListClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, Error> {
        let client = rest_client_builder()?
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .timeout(Duration::from_secs(READ_WRITE_TIMEOUT))
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.to_string(),
            forwarder: None,
        })
    }

    /// ERA fork: [`Self::new`] without a forwarder, otherwise a client that sends every request
    /// meant for `base_url` to `forwarder` (see [`Forwarder`]).
    pub(crate) fn new_routed(base_url: &str, forwarder: Option<&Forwarder>) -> Result<Self, Error> {
        let Some(forwarder) = forwarder else {
            return Self::new(base_url);
        };
        Ok(Self {
            client: forwarder.client()?,
            base_url: base_url.to_string(),
            forwarder: Some(forwarder.clone()),
        })
    }

    fn req_err(e: impl std::fmt::Display) -> Error {
        Error::RejectListService {
            details: e.to_string(),
        }
    }

    pub(crate) fn get_reject_list(&self) -> Result<String, Error> {
        // ERA fork: through the forwarder when one is set
        if let Some(forwarder) = &self.forwarder {
            return self.get_forwarded(forwarder);
        }
        self.client
            .get(&self.base_url)
            .send()
            .map_err(Self::req_err)?
            .text()
            .map_err(Self::req_err)
    }

    /// ERA fork: [`Self::get_reject_list`] through `forwarder`, failing closed.
    ///
    /// The caller reads the body as the list and skips every line that is not an opout, so any
    /// answer that is not the list itself (a forwarder refusing the host, an upstream that is
    /// down, an error page) would validate the asset against an empty list. Only a 2xx answer is
    /// read as the list. The forwarder's refusal is [`Error::ForwarderRefused`], anything else is
    /// [`Error::RejectListService`] with the status.
    fn get_forwarded(&self, forwarder: &Forwarder) -> Result<String, Error> {
        let response = forwarder
            .request(
                &self.client,
                reqwest::Method::GET,
                &self.base_url,
                FORWARD_KIND_REJECT_LIST,
                Self::req_err,
            )?
            .send()
            .map_err(Self::req_err)?;
        if let Some(refusal) = Forwarder::refusal(&response, &self.base_url) {
            return Err(refusal);
        }
        let status = response.status();
        if !status.is_success() {
            return Err(Error::RejectListService {
                details: format!("HTTP {status} from the forwarder"),
            });
        }
        response.text().map_err(Self::req_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_reject_list_error() {
        // network error
        let client = RejectListClient::new("http://127.0.0.1:1").unwrap();
        let result = client.get_reject_list().unwrap_err();
        assert_matches!(result, Error::RejectListService { .. });

        // content-length mismatch (truncated body)
        let mut server = mockito::Server::new();
        let mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-length", "100")
            .with_body(&[0xFFu8, 0xFEu8][..])
            .create();
        let client = RejectListClient::new(&server.url()).unwrap();
        let result = client.get_reject_list().unwrap_err();
        assert_matches!(result, Error::RejectListService { .. });
        mock.assert();
    }
}
