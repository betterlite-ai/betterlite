use betterlite_openai::fine_tuning::client::{
    BuildError, ClientArgs, ClientBuilder, JobsApi, SdkError,
};
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use std::time::Duration;

// the class the openai SDK raises for a response status
pub const fn error_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=599 => "InternalServerError",
        _ => "APIStatusError",
    }
}
pub fn sdk_error(status: u16, bytes: &[u8]) -> SdkError {
    let body: Value = serde_json::from_slice(bytes).unwrap_or(Value::Null);
    let field = |name: &str| {
        body.get("error")
            .and_then(|error| error.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    SdkError {
        status,
        error_type: field("type"),
        param: field("param"),
        code: field("code"),
        message: field("message").unwrap_or_default(),
    }
}
fn transport_error(status: u16, message: String) -> SdkError {
    SdkError {
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
// which header carries the credential: a key-based Azure client sends api-key, everything else a bearer token
#[derive(Debug, Clone, Copy)]
pub enum Auth {
    Bearer,
    ApiKey,
}
pub struct HttpJobsApi {
    pub is_async: bool,
    pub url: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub auth: Auth,
    pub timeout: Option<f64>,
}
fn headers(api_key: Option<&SecretString>, organization: Option<&str>, auth: Auth) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Some(key) = api_key {
        let (name, value) = match auth {
            Auth::Bearer => (
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", key.expose_secret()),
            ),
            Auth::ApiKey => (
                reqwest::header::HeaderName::from_static("api-key"),
                key.expose_secret().to_owned(),
            ),
        };
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    }
    if let Some(organization) = organization
        && let Ok(value) = HeaderValue::from_str(organization)
    {
        headers.insert("openai-organization", value);
    }
    headers
}
impl HttpJobsApi {
    pub fn send(&self, body: &Value) -> Result<Vec<u8>, SdkError> {
        let client = Client::builder()
            .no_proxy()
            .timeout(
                self.timeout
                    .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
            )
            .build()
            .map_err(|error| transport_error(0, error.to_string()))?;
        let response = client
            .post(&self.url)
            .headers(headers(
                self.api_key.as_ref(),
                self.organization.as_deref(),
                self.auth,
            ))
            .json(body)
            .send()
            .map_err(|error| transport_error(0, error.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .map_err(|error| transport_error(status, error.to_string()))?;
        if status >= 400 {
            return Err(sdk_error(status, &bytes));
        }
        Ok(bytes.to_vec())
    }
}
// the oracle only records the create request; the other operations never reach a mock
const ONLY_CREATE: &str = "only create reaches the fine-tuning oracle";
impl JobsApi for HttpJobsApi {
    fn is_async(&self) -> bool {
        self.is_async
    }
    fn create(&self, data: &FineTuneCreate) -> Result<Vec<u8>, SdkError> {
        // the openai SDK merges create.extra_body into the request body, so the wire body is the flattened record
        let mut body = serde_json::to_value(data).unwrap_or(Value::Null);
        if let Some(fields) = body.as_object_mut() {
            let merged = fields
                .remove("extra_body")
                .and_then(|value| value.as_object().cloned());
            if let Some(extra) = merged {
                fields.extend(extra);
            }
        }
        self.send(&body)
    }
    fn list(&self, _after: Option<&str>, _limit: Option<i64>) -> Result<Vec<u8>, SdkError> {
        Err(transport_error(0, ONLY_CREATE.to_owned()))
    }
    fn retrieve(&self, _job_id: &str) -> Result<Vec<u8>, SdkError> {
        Err(transport_error(0, ONLY_CREATE.to_owned()))
    }
    fn cancel(&self, _job_id: &str) -> Result<Vec<u8>, SdkError> {
        Err(transport_error(0, ONLY_CREATE.to_owned()))
    }
}
// the production builder belongs to the SDK dispatcher; the oracle only needs the real HTTP client
pub struct HttpBuilder;
impl ClientBuilder for HttpBuilder {
    fn build(
        &self,
        is_async: bool,
        args: &ClientArgs,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        let Some(base_url) = args.base_url.clone() else {
            return Err(BuildError::UnexpectedArgument {
                name: "base_url".to_owned(),
            });
        };
        Ok(Some(Box::new(HttpJobsApi {
            is_async,
            url: format!("{}/fine_tuning/jobs", base_url.trim_end_matches('/')),
            api_key: args.api_key.clone(),
            organization: args.organization.clone(),
            auth: Auth::Bearer,
            timeout: args.timeout,
        })))
    }
}
