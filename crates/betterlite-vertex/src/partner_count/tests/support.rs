#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::credential_sources::getters::VertexParams;
use crate::partner_count::body::PartnerCountBody;
use crate::partner_count::execute::PartnerCountTransport;
use crate::partner_count::options::PartnerCountOptions;
use crate::token_cache::probe::AccessToken;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

#[derive(Clone, Debug)]
pub(super) struct Sent {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) deadline: Duration,
}

// Answers every request with the same reply and records each request, so a test sees what went out
#[derive(Clone, Default)]
pub(super) struct Script {
    reply: Option<(u16, Vec<u8>)>,
    sent: Arc<Mutex<Vec<Sent>>>,
}

impl Script {
    pub(super) fn answering(status: u16, body: &str) -> Self {
        Self::answering_bytes(status, body.as_bytes().to_vec())
    }

    pub(super) fn answering_bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            reply: Some((status, body)),
            sent: Arc::default(),
        }
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent.lock().unwrap().clone()
    }
}

impl PartnerCountTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let payload = match request.body {
            RequestBody::Json(bytes) => bytes,
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Vec::new()
            }
        };
        self.sent.lock().unwrap().push(Sent {
            method: request.method,
            url: request.url.to_string(),
            headers: request.headers,
            body: payload,
            deadline: request.deadline,
        });
        ready(match &self.reply {
            Some((status, body)) => Ok(UpstreamResponse {
                status: *status,
                headers: HeaderMap::new(),
                body: body.clone(),
            }),
            None => Err(TransportError::Closed),
        })
    }
}

pub(super) struct FixedMint;

impl MintAccessToken for FixedMint {
    fn mint<'mint>(
        &'mint self,
        _credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(ready(Ok(access(project))))
    }
}

// Keeps the credential each mint received, so a test can prove an overlay reached the lookup
#[derive(Default)]
pub(super) struct RecordingMint {
    seen: Mutex<Vec<Option<String>>>,
    projects: Mutex<Vec<Option<String>>>,
}

impl RecordingMint {
    pub(super) fn seen(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }

    pub(super) fn projects(&self) -> Vec<Option<String>> {
        self.projects.lock().unwrap().clone()
    }
}

impl MintAccessToken for RecordingMint {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        self.seen.lock().unwrap().push(
            credentials
                .and_then(ConfiguredCredentials::as_text)
                .map(str::to_owned),
        );
        self.projects
            .lock()
            .unwrap()
            .push(project.map(str::to_owned));
        Box::pin(ready(Ok(access(project))))
    }
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

pub(super) fn body(model: Option<&str>, messages: Option<Vec<Json>>) -> PartnerCountBody {
    PartnerCountBody {
        model: model.map(str::to_owned),
        messages,
        extra: IndexMap::new(),
    }
}

pub(super) fn options(count_tokens_location: Option<&str>) -> PartnerCountOptions {
    PartnerCountOptions {
        vertex_count_tokens_location: count_tokens_location.map(str::to_owned),
    }
}

pub(super) fn settings(location: Option<&str>) -> VertexSettings {
    VertexSettings {
        project: None,
        location: location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
    }
}

pub(super) fn params<'call>(
    project: Option<&'call str>,
    location: Option<&'call str>,
    credentials: Option<&'call ConfiguredCredentials>,
) -> VertexParams<'call> {
    VertexParams {
        project,
        ai_project: None,
        location,
        ai_location: None,
        credentials,
        ai_credentials: None,
    }
}

pub(super) fn access(project: Option<&str>) -> AccessToken {
    AccessToken {
        token: SecretString::from("minted"),
        project: project.unwrap_or("minted-project").to_owned(),
    }
}

pub(super) fn credentials(text: &str) -> ConfiguredCredentials {
    ConfiguredCredentials::Text(SecretString::from(text.to_owned()))
}
