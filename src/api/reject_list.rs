use super::{
    forwarder::{FORWARD_KIND_REJECT_LIST, Forwarder},
    *,
};

/// Maximum size, in bytes, of a reject list line: the allow prefix (`!`), the opout
/// (`{opid}/{ty}/{no}`, a hex opid and two decimal `u16`s) and a CRLF line ending
const MAX_REJECT_LIST_LINE_SIZE: u64 = 1 + 64 + 1 + 5 + 1 + 5 + 2;
/// Maximum size, in bytes, of a reject list served at a URL (see [`MAX_REJECT_LIST_ENTRIES`])
const MAX_REJECT_LIST_SIZE: u64 = MAX_REJECT_LIST_ENTRIES * MAX_REJECT_LIST_LINE_SIZE;

pub struct RejectListClient {
    client: RestClient,
    base_url: String,
    // ERA fork: when set, requests go to this loopback forwarder instead of base_url
    forwarder: Option<Forwarder>,
}

impl RejectListClient {
    /// A client for the reject list served at `base_url`.
    ///
    /// The URL is taken from the asset's genesis, so it's untrusted: only `http` and `https` URLs
    /// are accepted and redirects are not followed, to not let it make the client request other
    /// URLs (e.g. on the local network) than the one it states.
    pub(crate) fn new(base_url: &str) -> Result<Self, Error> {
        let url = reqwest::Url::parse(base_url).map_err(Self::req_err)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Self::req_err(format!(
                "unsupported reject list URL scheme: {}",
                url.scheme()
            )));
        }
        let client = rest_client_builder()?
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
            .timeout(Duration::from_secs(READ_WRITE_TIMEOUT))
            .redirect(reqwest::redirect::Policy::none())
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
        let resp = self
            .client
            .get(&self.base_url)
            .send()
            .map_err(Self::req_err)?;
        Self::read_list(resp)
    }

    /// The reject list in `resp`, refused when it's not one (upstream's checks, shared by the
    /// direct and the forwarded request).
    fn read_list(resp: Response) -> Result<String, Error> {
        // an error page (or a redirect, not followed) is not a reject list: taking it as one would
        // make every opout look allowed
        let status = resp.status();
        if !status.is_success() {
            return Err(Self::req_err(format!(
                "reject list request failed: HTTP {status}"
            )));
        }
        // read one byte past the bound to tell a body of exactly the maximum size from a longer one
        let mut body = vec![];
        resp.take(MAX_REJECT_LIST_SIZE + 1)
            .read_to_end(&mut body)
            .map_err(Self::req_err)?;
        if body.len() as u64 > MAX_REJECT_LIST_SIZE {
            return Err(Self::req_err(format!(
                "reject list too large: more than {MAX_REJECT_LIST_SIZE} bytes"
            )));
        }
        // a reject list is made of opouts, which are ASCII: a lossy conversion of anything else
        // could take up to 3 times the size bound, each invalid byte becoming U+FFFD
        if !body.is_ascii() {
            return Err(Self::req_err("reject list is not ASCII"));
        }
        Ok(String::from_utf8(body).expect("ASCII is valid UTF-8"))
    }

    /// ERA fork: [`Self::get_reject_list`] through `forwarder`, failing closed.
    ///
    /// The caller reads the body as the list and skips every line that is not an opout, so any
    /// answer that is not the list itself (a forwarder refusing the host, an upstream that is
    /// down, an error page) would validate the asset against an empty list. Only a 200 answer
    /// that is empty or holds at least one opout is read as the list. The forwarder's refusal is
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
        let list = Self::read_list(response)?;
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

        // error pages and redirects (not followed) are refused, not taken as an empty list
        for status in [301, 302, 404, 500] {
            let mut server = mockito::Server::new();
            let mock = server
                .mock("GET", "/")
                .with_status(status)
                .with_header("location", "http://127.0.0.1:1/elsewhere")
                .with_body("not a reject list")
                .create();
            let client = RejectListClient::new(&server.url()).unwrap();
            let result = client.get_reject_list().unwrap_err();
            assert_matches!(result, Error::RejectListService { ref details } if details.contains(&status.to_string()));
            mock.assert();
        }

        // a body that is not ASCII is refused
        let mut server = mockito::Server::new();
        let mock = server.mock("GET", "/").with_body("é").create();
        let client = RejectListClient::new(&server.url()).unwrap();
        let result = client.get_reject_list().unwrap_err();
        assert_matches!(result, Error::RejectListService { ref details } if details == "reject list is not ASCII");
        mock.assert();

        // URLs other than http(s) ones are refused
        for url in ["file:///etc/passwd", "ftp://127.0.0.1/list", "not a url"] {
            assert_matches!(
                RejectListClient::new(url).err(),
                Some(Error::RejectListService { .. })
            );
        }
    }

    #[test]
    fn reject_list_line_size() {
        // the longest line is an allowed opout with the largest assignment type and output number
        let opout = Opout::new(
            OpId::from([0xff; 32]),
            AssignmentType::with(u16::MAX),
            u16::MAX,
        );
        let line = format!("!{opout}\r\n");
        assert_eq!(line.len() as u64, MAX_REJECT_LIST_LINE_SIZE);
        assert_eq!(Opout::from_str(line[1..].trim_end()).unwrap(), opout);
    }

    #[test]
    fn get_reject_list_size() {
        // a list of the maximum size is read
        let mut server = mockito::Server::new();
        let body = "a".repeat(MAX_REJECT_LIST_SIZE as usize);
        let mock = server.mock("GET", "/").with_body(&body).create();
        let client = RejectListClient::new(&server.url()).unwrap();
        assert_eq!(client.get_reject_list().unwrap(), body);
        mock.assert();

        // a longer one is refused, with or without a content length
        for chunked in [false, true] {
            let mut server = mockito::Server::new();
            let body = "a".repeat(MAX_REJECT_LIST_SIZE as usize + 1);
            let mock = if chunked {
                server
                    .mock("GET", "/")
                    .with_chunked_body(move |w| w.write_all(body.as_bytes()))
            } else {
                server.mock("GET", "/").with_body(&body)
            }
            .create();
            let client = RejectListClient::new(&server.url()).unwrap();
            let result = client.get_reject_list().unwrap_err();
            assert_matches!(result, Error::RejectListService { ref details } if details.contains("reject list too large"));
            mock.assert();
        }
    }
}
