use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::records::{BinaryContent, FileDeleted, FileObject};
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::SecretString;

use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use crate::files::api::FilesApi;
use crate::files::error::FilesError;
use crate::files::request::{FileCall, FilesClient, ListCall, Upload};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const TIMEOUT: Duration = Duration::from_secs(7);

pub(super) struct Fixture {
    pub(super) status: u16,
    pub(super) headers: Vec<(&'static str, &'static str)>,
    pub(super) body: Vec<u8>,
    pub(super) fail: bool,
    pub(super) seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    pub(super) fn answering(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_vec(),
            fail: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn answering_with(
        status: u16,
        headers: &[(&'static str, &'static str)],
        body: &[u8],
    ) -> Self {
        let mut fixture = Self::answering(status, body);
        fixture.headers = headers.to_vec();
        fixture
    }

    pub(super) fn failing() -> Self {
        Self {
            status: 0,
            headers: Vec::new(),
            body: Vec::new(),
            fail: true,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reply(&self) -> UpstreamResponse {
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }
        UpstreamResponse {
            status: self.status,
            headers,
            body: self.body.clone(),
        }
    }

    pub(super) fn only_request(&self) -> OutboundRequest {
        let mut seen = self.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "the operation sends exactly one request");
        seen.remove(0)
    }

    pub(super) fn sent(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

impl FileTransport for &Fixture {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.lock().unwrap().push(request);
        if self.fail {
            return ready(Err(TransportError::Closed));
        }
        ready(Ok(self.reply()))
    }

    fn send_stream(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        ready(Err(TransportError::Closed))
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn configured(
    api_key: Option<&str>,
    auth_token: Option<&str>,
    api_base: Option<&str>,
) -> AnthropicSettings {
    AnthropicSettings {
        api_key: api_key.map(secret),
        auth_token: auth_token.map(secret),
        api_base: api_base.map_or_else(|| DEFAULT_API_BASE.to_owned(), str::to_owned),
        use_bearer_for_custom_base: false,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    }
}

pub(super) fn client(api_key: Option<&str>, api_base: Option<&str>) -> FilesClient {
    let settings = configured(None, None, None);
    FilesClient {
        api_key: api_key.map(secret),
        api_base: api_base.map(str::to_owned),
        settings,
        timeout: TIMEOUT,
    }
}

fn api(transport: &Fixture) -> FilesApi<&Fixture> {
    FilesApi {
        transport,
        clock: || NOW,
    }
}

pub(super) async fn upload(
    transport: &Fixture,
    client: &FilesClient,
    call: &Upload<'_>,
) -> Result<FileObject, FilesError> {
    api(transport).create_file(client, call).await
}

pub(super) async fn retrieve(
    transport: &Fixture,
    client: &FilesClient,
    call: &FileCall<'_>,
) -> Result<FileObject, FilesError> {
    api(transport).retrieve_file(client, call).await
}

pub(super) async fn delete(
    transport: &Fixture,
    client: &FilesClient,
    call: &FileCall<'_>,
) -> Result<FileDeleted, FilesError> {
    api(transport).delete_file(client, call).await
}

pub(super) async fn list(
    transport: &Fixture,
    client: &FilesClient,
    call: &ListCall<'_>,
) -> Result<Vec<FileObject>, FilesError> {
    api(transport).list_files(client, call).await
}

pub(super) async fn content(
    transport: &Fixture,
    client: &FilesClient,
    call: &FileCall<'_>,
) -> Result<BinaryContent, FilesError> {
    api(transport).content(client, call).await
}
