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
pub(crate) struct Input {
    pub(crate) client: ClientSpec,
    pub(crate) request: Request,
    #[serde(default)]
    pub(crate) reply: Option<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClientSpec {
    pub(crate) timeout: u64,
    pub(crate) max_retries: u32,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) organization: Option<String>,
    #[serde(default)]
    pub(crate) project: Option<String>,
    #[serde(default)]
    pub(crate) aspeech: Option<bool>,
    #[serde(default)]
    pub(crate) supplied: Option<SuppliedKind>,
    #[serde(default)]
    pub(crate) shared_session: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SuppliedKind {
    Sync,
    Async,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: String,
    #[serde(default)]
    pub(crate) response_format: Option<SpeechFormat>,
    #[serde(default)]
    pub(crate) speed: Option<Decimal>,
    #[serde(default)]
    pub(crate) instructions: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: IndexMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) chunks: Vec<Vec<u8>>,
    pub(crate) text: Option<String>,
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
