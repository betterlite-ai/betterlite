use super::input::{ReplyCredentials, WebIdentityReply};
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
use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::sts::{StsClientKwargs, StsSettings, sts_client_kwargs};
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_config::tls::Verification;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};
use url::form_urlencoded;
#[derive(Clone, Debug, Default)]
pub(super) struct Record(Arc<Mutex<Vec<Value>>>);
impl Record {
    pub(super) fn add(&self, call: Value) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
    pub(super) fn calls(&self) -> Vec<Value> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
pub(super) type Answer = (u16, String);
#[derive(Clone, Debug)]
pub(super) struct Transport {
    pub(super) answers: BTreeMap<String, Answer>,
    pub(super) record: Record,
}
fn refused(message: &'static str) -> HttpConnectorFuture {
    HttpConnectorFuture::ready(Err(ConnectorError::other(
        Box::new(io::Error::other(message)),
        None,
    )))
}
fn access_key(request: &HttpRequest) -> Option<String> {
    let authorization = request.headers().get("authorization")?;
    let credential = authorization.split("Credential=").nth(1)?;
    credential.split('/').next().map(str::to_owned)
}
fn form_fields(request: &HttpRequest) -> BTreeMap<String, String> {
    let body = request.body().bytes().unwrap_or_default();
    form_urlencoded::parse(body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}
impl HttpConnector for Transport {
    fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
        let fields = form_fields(&request);
        self.record.add(json!({
            "form": fields,
            "access_key_id": access_key(&request),
            "session_token": request.headers().get("x-amz-security-token"),
        }));
        let Some(answer) = fields
            .get("Action")
            .and_then(|action| self.answers.get(action))
        else {
            return refused("the fixture has no answer for this action");
        };
        let Ok(status) = StatusCode::try_from(answer.0) else {
            return refused("the fixture answer status is invalid");
        };
        HttpConnectorFuture::ready(Ok(HttpResponse::new(
            status,
            SdkBody::from(answer.1.clone()),
        )))
    }
}
pub(super) fn sources(
    environment: &BTreeMap<String, String>,
    files: &[(&str, &str)],
    transport: Transport,
) -> NativeSources {
    NativeSources {
        environment: environment.clone(),
        filesystem: Fs::from_slice(files),
        client: http_client_fn(move |_, _| SharedHttpConnector::new(transport.clone())),
        clock: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    }
}
pub(super) fn kwargs(
    environment: &BTreeMap<String, String>,
) -> Result<StsClientKwargs, AwsEndpointError> {
    let settings = StsSettings {
        endpoint: None,
        aws_region: environment.get("AWS_REGION").map(String::as_str),
        aws_default_region: environment.get("AWS_DEFAULT_REGION").map(String::as_str),
        configured_region: None,
    };
    sts_client_kwargs(&settings, Verification::Enabled)
}
fn credentials_xml(credentials: &ReplyCredentials) -> String {
    format!(
        "<Credentials><AccessKeyId>{}</AccessKeyId><SecretAccessKey>{}</SecretAccessKey><SessionToken>{}</SessionToken><Expiration>{}</Expiration></Credentials>",
        credentials.access_key_id,
        credentials.secret_access_key,
        credentials.session_token,
        credentials.expiration
    )
}
fn packed_xml(size: Option<i32>) -> String {
    size.map(|value| format!("<PackedPolicySize>{value}</PackedPolicySize>"))
        .unwrap_or_default()
}
fn error_xml(code: &str, message: &str) -> String {
    format!(
        "<ErrorResponse><Error><Type>Sender</Type><Code>{code}</Code><Message>{message}</Message></Error><RequestId>oracle</RequestId></ErrorResponse>"
    )
}
pub(super) fn denied_identity() -> Answer {
    (
        403,
        error_xml("AccessDenied", "the fixture identity call is denied"),
    )
}
pub(super) fn web_identity_answer(reply: &WebIdentityReply) -> Answer {
    match reply {
        WebIdentityReply::Credentials(credentials) => (
            200,
            format!(
                "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                credentials_xml(credentials),
                packed_xml(credentials.packed_policy_size)
            ),
        ),
        WebIdentityReply::MissingPackedPolicySize(credentials) => (
            200,
            format!(
                "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult>{}</AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
                credentials_xml(credentials)
            ),
        ),
        WebIdentityReply::InvalidIdentityToken { message } => {
            (400, error_xml("InvalidIdentityToken", message))
        }
    }
}
pub(super) fn assume_role_answer(credentials: &ReplyCredentials) -> Answer {
    (
        200,
        format!(
            "<AssumeRoleResponse><AssumeRoleResult>{}{}</AssumeRoleResult></AssumeRoleResponse>",
            credentials_xml(credentials),
            packed_xml(credentials.packed_policy_size)
        ),
    )
}
