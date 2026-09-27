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
    /// down, an error page, a list cut short) would validate the asset against an empty or partial
    /// list. Only a 200 answer whose body has a checked end (a length or chunked framing) and is
    /// empty or holds at least one opout is read as the list. The forwarder's refusal is
    /// [`Error::ForwarderRefused`], anything else is [`Error::RejectListService`].
    fn get_forwarded(&self, forwarder: &Forwarder) -> Result<String, Error> {
        // an error names the list, not the forwarder (see Forwarder::scrub)
        let scrubbed = |e: reqwest::Error| Self::req_err(Forwarder::scrub(e, &self.base_url));
        let response = forwarder
            .request(
                &self.client,
                reqwest::Method::GET,
                &self.base_url,
                FORWARD_KIND_REJECT_LIST,
                Self::req_err,
            )?
            .send()
            .map_err(scrubbed)?;
        if let Some(refusal) = forwarder.refusal(&response, &self.base_url) {
            return Err(refusal);
        }
        let status = response.status();
        // only a 200 is the list: a 203, 204 or 206 is not the list as its issuer serves it
        if status != reqwest::StatusCode::OK {
            return Err(Error::RejectListService {
                details: format!("HTTP {status} from the forwarder"),
            });
        }
        // and only a whole one: a body that the connection's close ends cannot be told from one
        // cut short (after its first line, say)
        if !has_checked_end(&response) {
            return Err(Error::RejectListService {
                details: s!("the forwarded answer has neither a length nor chunked framing"),
            });
        }
        let list = response.text().map_err(scrubbed)?;
        // and text that holds no opout at all is not a list either (an error page passed on with
        // a 200); an empty answer is an empty list
        let lines = list.trim();
        if !lines.is_empty()
            && !lines
                .lines()
                .any(|line| Opout::from_str(line.strip_prefix('!').unwrap_or(line)).is_ok())
        {
            return Err(Error::RejectListService {
                details: s!("the forwarded answer holds no opout"),
            });
        }
        Ok(list)
    }
}

/// ERA fork: whether the body of `response` has an end the client checks, so that reading it
/// fails if the body is cut short: a `Content-Length` (hyper fails on a shorter body), chunked as
/// the last transfer coding (it fails without the last chunk), or HTTP/2 framing. An HTTP/1 body
/// without either ends where the connection closes, wherever that is.
fn has_checked_end(response: &reqwest::blocking::Response) -> bool {
    if response.version() >= reqwest::Version::HTTP_2 {
        return true;
    }
    let headers = response.headers();
    if headers.contains_key(reqwest::header::CONTENT_LENGTH) {
        return true;
    }
    // the codings of every Transfer-Encoding line, in order: chunked must be the last one
    let codings: Vec<String> = headers
        .get_all(reqwest::header::TRANSFER_ENCODING)
        .iter()
        .flat_map(|value| {
            String::from_utf8_lossy(value.as_bytes())
                .split(',')
                .map(|coding| coding.trim().to_ascii_lowercase())
                .filter(|coding| !coding.is_empty())
                .collect::<Vec<_>>()
        })
        .collect();
    codings.last().is_some_and(|coding| coding == "chunked")
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
