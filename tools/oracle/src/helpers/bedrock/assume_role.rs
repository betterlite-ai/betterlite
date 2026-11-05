use crate::helpers::{Helper, HelperError, HelperResult};
use aws_smithy_async::{
    rt::sleep::{SharedAsyncSleep, TokioSleep},
    time::{SharedTimeSource, StaticTimeSource},
};
use aws_smithy_runtime_api::client::{
    http::{HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn},
    orchestrator::{HttpRequest, HttpResponse},
    result::ConnectorError,
};
use aws_smithy_runtime_api::http::StatusCode;
use aws_smithy_types::body::SdkBody;
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::assume_role::{
    expiry::{StsResponse, extract_credentials_and_ttl},
    is_already_running_as_role, parse_role_arn,
};
use betterlite_bedrock::aws_settings::sources::NativeSources;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{json, value::RawValue};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{collections::BTreeMap, fmt::Display, io, sync::Arc, time::Duration, time::UNIX_EPOCH};

mod tests;

const IDENTITY: &str = "<GetCallerIdentityResponse><GetCallerIdentityResult>";
const IDENTITY_END: &str = "</GetCallerIdentityResult></GetCallerIdentityResponse>";
const DENIED: &str = "<ErrorResponse><Error><Code>AccessDenied</Code></Error></ErrorResponse>";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reply {
    Identity(String),
    NoArn,
    Denied,
    Unreachable,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Probe {
    environment: BTreeMap<String, String>,
    target: String,
    reply: Reply,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Extract {
    sts_response: Box<RawValue>,
    now: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    ParseArn { arn: String },
    AlreadyRunning(Probe),
    ExtractTtl(Extract),
}

#[derive(Clone, Debug)]
struct Canned {
    reply: Option<(StatusCode, String)>,
    identity_calls: Arc<AtomicUsize>,
    assume_calls: Arc<AtomicUsize>,
}

impl HttpConnector for Canned {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let body = request.body().bytes().unwrap_or_default();
        if body.starts_with(b"Action=GetCallerIdentity") {
            self.identity_calls.fetch_add(1, Ordering::SeqCst);
        } else {
            self.assume_calls.fetch_add(1, Ordering::SeqCst);
        }
        let Some((status, xml)) = self.reply.clone() else {
            let refused = io::Error::other("fixture unreachable");
            return HttpConnectorFuture::ready(Err(ConnectorError::other(Box::new(refused), None)));
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(status, SdkBody::from(xml))))
    }
}

fn probe(input: &Probe) -> HelperResult {
    let ok = StatusCode::try_from(200_u16).map_err(failure)?;
    let reply = match &input.reply {
        Reply::Identity(arn) => Some((ok, format!("{IDENTITY}<Arn>{arn}</Arn>{IDENTITY_END}"))),
        Reply::NoArn => Some((ok, format!("{IDENTITY}{IDENTITY_END}"))),
        Reply::Denied => Some((
            StatusCode::try_from(403_u16).map_err(failure)?,
            DENIED.to_owned(),
        )),
        Reply::Unreachable => None,
    };
    let canned = Canned {
        reply,
        identity_calls: Arc::default(),
        assume_calls: Arc::default(),
    };
    let shared = canned.clone();
    let sources = NativeSources {
        environment: input.environment.clone(),
        filesystem: Fs::from_slice(&[]),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(shared.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let matches = runtime.block_on(is_already_running_as_role(&input.target, &sources));
    Ok(json!({
        "matches": matches,
        "sts_calls": canned.identity_calls.load(Ordering::SeqCst),
        "assume_role_calls": canned.assume_calls.load(Ordering::SeqCst),
    }))
}

fn extract(input: &Extract) -> HelperResult {
    let response: StsResponse = serde_json::from_str(input.sts_response.get()).map_err(failure)?;
    let (credentials, ttl) = extract_credentials_and_ttl(response, input.now).map_err(failure)?;
    Ok(json!({
        "credentials": [
            credentials.access_key_id.expose_secret(),
            credentials.secret_access_key.expose_secret(),
            credentials.session_token.map(|token| token.expose_secret().to_owned()),
        ],
        "ttl": ttl,
    }))
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "AssumeRoleError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ParseArn { arn } => {
                let found = parse_role_arn(arn).map(|id| [id.partition, id.account, id.role]);
                Ok(json!({"value": found}))
            }
            Input::AlreadyRunning(request) => probe(request),
            Input::ExtractTtl(request) => extract(request),
        }
    }
}
