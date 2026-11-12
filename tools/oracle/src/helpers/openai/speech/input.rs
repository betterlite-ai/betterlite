use betterlite_openai::speech::client::{
    ClientKind, SharedSession, SpeechConnection, SuppliedClient, normalize_base,
};
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::{SpeechFormat, SpeechOptions};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub(super) client: ClientSpec,
    pub(super) request: Request,
    #[serde(default)]
    pub(super) reply: Option<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientSpec {
    pub(super) timeout: u64,
    pub(super) max_retries: u32,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) organization: Option<String>,
    #[serde(default)]
    pub(super) project: Option<String>,
    #[serde(default)]
    pub(super) aspeech: Option<bool>,
    #[serde(default)]
    pub(super) supplied: Option<SuppliedKind>,
    #[serde(default)]
    pub(super) shared_session: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SuppliedKind {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub(super) model: String,
    pub(super) input: String,
    pub(super) voice: String,
    #[serde(default)]
    pub(super) response_format: Option<SpeechFormat>,
    #[serde(default)]
    pub(super) speed: Option<Decimal>,
    #[serde(default)]
    pub(super) instructions: Option<String>,
    #[serde(default)]
    pub(super) extra_headers: IndexMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) chunks: Vec<Vec<u8>>,
    pub(super) text: Option<String>,
}

pub(super) fn options(request: &Request) -> SpeechOptions {
    SpeechOptions {
        response_format: request.response_format,
        speed: request.speed,
        instructions: request.instructions.clone(),
        extra_headers: request.extra_headers.clone(),
    }
}

pub(super) fn connection(spec: &ClientSpec) -> Result<SpeechConnection, SpeechError> {
    let supplied = match spec.supplied {
        Some(kind) => Some(SuppliedClient {
            kind: match kind {
                SuppliedKind::Sync => ClientKind::Sync,
                SuppliedKind::Async => ClientKind::Async,
            },
            api_key: Some(SecretString::from("sk-supplied")),
            base_url: normalize_base("https://supplied.test/v1")?,
            timeout: Duration::from_secs(45),
            max_retries: 1,
            organization: Some(String::from("org-supplied")),
        }),
        None => None,
    };
    Ok(SpeechConnection {
        api_key: spec.api_key.clone().map(SecretString::from),
        api_base: spec.api_base.clone(),
        timeout: Duration::from_secs(spec.timeout),
        max_retries: spec.max_retries,
        organization: spec.organization.clone(),
        project: spec.project.clone(),
        client: supplied,
        shared_session: spec.shared_session.then_some(SharedSession),
    })
}
