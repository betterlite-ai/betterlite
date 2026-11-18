use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_smithy_async::time::SharedTimeSource;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use futures_util::{Stream, StreamExt as _};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use url::Url;

use crate::aws_settings::pop_aws_auth_params;
use crate::aws_settings::sources::Credentials;
use crate::credentials::AwsCredentialResolver;
use crate::credentials::error::CredentialError;
use crate::signing::error::SigningError;
use crate::signing::target::{RegionInputs, auth_settings};

use super::endpoint::ParamMap;
use super::error::RuntimeError;

pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, HttpError>> + Send>>;

const SECRET_HEADERS: [&str; 2] = ["authorization", "x-amz-security-token"];
// an error body is read only to report it, so a runaway reply cannot hold the call open past this
const ERROR_BODY_BYTES: usize = 64 * 1024;

// the environment reads B682 resolves; the runtime never touches the process environment itself
#[derive(Debug, Clone, Default)]
pub struct SagemakerEnvironment {
    pub region_name: Option<String>,
    pub region: Option<String>,
    pub session_region: Option<String>,
}

impl SagemakerEnvironment {
    pub(crate) const fn inputs(&self) -> RegionInputs<'_> {
        RegionInputs {
            environment_region_name: self.region_name.as_deref(),
            environment_region: self.region.as_deref(),
            session_region: self.session_region.as_deref(),
        }
    }
}

pub trait SagemakerTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;

    fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<(u16, ChunkStream), TransportError>> + Send;
}

impl SagemakerTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx clients follow redirects, so each hop is checked the same way
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }

    async fn open_stream(
        &self,
        request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        let response = self.send_following(request).await?;
        let status = response.response.status().as_u16();
        Ok((status, response.body_stream()))
    }
}

pub trait SagemakerCredentials: Send + Sync {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

impl SagemakerCredentials for AwsCredentialResolver<'_> {
    async fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> Result<Credentials, CredentialError> {
        self.resolve_credentials(&settings, Some(&region)).await
    }
}

pub struct SagemakerRuntime<'deps, T, C> {
    pub transport: T,
    pub credentials: C,
    pub environment: SagemakerEnvironment,
    pub clock: SharedTimeSource,
    pub counter: &'deps (dyn TextTokenCounter + Sync),
    pub timeout: Duration,
}

// the signed call as it goes on the wire, with the body kept as an object for the log
pub struct PreparedCall {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub body_json: IndexMap<String, Json>,
    pub region: String,
}

impl PreparedCall {
    // the pre_call record LiteLLM fills for a completion: the signed url, headers and body
    #[must_use]
    pub fn additional_args(&self) -> AdditionalArgs {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect();
        AdditionalArgs {
            api_base: Some(self.url.clone()),
            headers: Some(headers),
            complete_input_dict: Some(RawBody::Object(self.body_json.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        }
    }
}

impl<T, C> SagemakerRuntime<'_, T, C> {
    pub(crate) fn now(&self) -> SystemTime {
        self.clock.now()
    }

    pub(crate) fn inputs(&self) -> RegionInputs<'_> {
        self.environment.inputs()
    }

    fn outbound(&self, prepared: &PreparedCall) -> Result<OutboundRequest, RuntimeError> {
        Ok(OutboundRequest {
            method: Method::POST,
            url: Url::parse(&prepared.url).map_err(|_parse| RuntimeError::Endpoint {
                endpoint: prepared.url.clone(),
            })?,
            query: Vec::new(),
            headers: header_map(&prepared.headers)?,
            body: RequestBody::Json(prepared.body.clone()),
            deadline: self.timeout,
        })
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    C: SagemakerCredentials,
{
    // _load_credentials: the aws_* arguments are popped and the credential source resolves them
    pub(crate) async fn credentials_for(
        &self,
        params: &ParamMap,
        region: &str,
    ) -> Result<Credentials, RuntimeError> {
        let mut copy = params.clone();
        let auth = pop_aws_auth_params(&mut copy).map_err(SigningError::from)?;
        let settings = auth_settings(auth, region);
        Ok(self
            .credentials
            .resolve(settings, region.to_owned())
            .await?)
    }
}

impl<T, C> SagemakerRuntime<'_, T, C>
where
    T: SagemakerTransport,
{
    pub(crate) async fn post(
        &self,
        prepared: &PreparedCall,
    ) -> Result<UpstreamResponse, RuntimeError> {
        Ok(self.transport.send(self.outbound(prepared)?).await?)
    }

    pub(crate) async fn open(
        &self,
        prepared: &PreparedCall,
    ) -> Result<(u16, ChunkStream), RuntimeError> {
        Ok(self.transport.open_stream(self.outbound(prepared)?).await?)
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap, RuntimeError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let (Ok(header_name), Ok(mut header_value)) = (
            HeaderName::try_from(name.as_str()),
            HeaderValue::try_from(value.as_str()),
        ) else {
            return Err(RuntimeError::Header { name: name.clone() });
        };
        header_value.set_sensitive(SECRET_HEADERS.contains(&header_name.as_str()));
        map.append(header_name, header_value);
    }
    Ok(map)
}

// the wire reply LiteLLM turns into SagemakerError(status_code, message)
#[must_use]
pub fn wire_error(status: u16, body: &[u8]) -> SagemakerError {
    SagemakerError {
        status_code: status,
        message: body_text(body),
        headers: None,
    }
}

// a failed stream still carries its body, which LiteLLM cannot read on an unread response
pub async fn stream_error(status: u16, mut chunks: ChunkStream) -> SagemakerError {
    let mut body = Vec::new();
    while body.len() < ERROR_BODY_BYTES {
        match chunks.next().await {
            Some(Ok(chunk)) => {
                let remaining = ERROR_BODY_BYTES - body.len();
                body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            }
            Some(Err(_)) | None => break,
        }
    }
    wire_error(status, &body)
}

// dict(response.headers) for the chat error class
#[must_use]
pub fn reply_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

// LiteLLM reads response.text, which decodes any bytes; betterlite refuses bytes that are not UTF-8
#[must_use]
pub fn body_text(body: &[u8]) -> String {
    std::str::from_utf8(body).map_or_else(
        |error| format!("{} bytes that are not UTF-8", error.as_bytes().len()),
        str::to_owned,
    )
}

#[must_use]
pub fn epoch_seconds(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{
        OutboundRequest, PreparedCall, ProxiedClient, RequestBody, SagemakerEnvironment,
        SagemakerRuntime, SagemakerTransport, header_map,
    };
    use crate::sagemaker_runtime::error::RuntimeError;
    use betterlite_logging::logging_pre_post::args::RawBody;
    use betterlite_models::decisions::Json;
    use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
    use indexmap::IndexMap;
    use std::time::Duration;

    struct Counter;

    impl TextTokenCounter for Counter {
        fn count_response_tokens(&self, text: &str) -> u64 {
            u64::try_from(text.split_whitespace().count()).unwrap_or_default()
        }
    }

    #[test]
    fn environment_inputs_keep_each_region_source() {
        let environment = SagemakerEnvironment {
            region_name: Some(String::from("eu-west-1")),
            region: Some(String::from("us-east-1")),
            session_region: Some(String::from("ap-south-1")),
        };
        let inputs = environment.inputs();
        assert_eq!(inputs.environment_region_name, Some("eu-west-1"));
        assert_eq!(inputs.environment_region, Some("us-east-1"));
        assert_eq!(inputs.session_region, Some("ap-south-1"));
        assert_eq!(
            SagemakerEnvironment::default().inputs().environment_region,
            None
        );
    }

    #[test]
    fn runtime_reads_its_injected_clock_and_environment() {
        let counter = Counter;
        let runtime = SagemakerRuntime {
            transport: (),
            credentials: (),
            environment: SagemakerEnvironment {
                region_name: Some(String::from("eu-west-1")),
                region: Some(String::from("us-east-1")),
                session_region: None,
            },
            clock: aws_smithy_async::time::SharedTimeSource::new(
                aws_smithy_async::time::StaticTimeSource::new(
                    std::time::UNIX_EPOCH + Duration::from_secs(12),
                ),
            ),
            counter: &counter,
            timeout: Duration::from_secs(3),
        };
        assert_eq!(
            runtime.now(),
            std::time::UNIX_EPOCH + Duration::from_secs(12)
        );
        assert_eq!(runtime.inputs().environment_region_name, Some("eu-west-1"));
        assert_eq!(runtime.inputs().environment_region, Some("us-east-1"));
    }

    #[test]
    fn header_map_validates_names_and_values_and_marks_secrets() {
        let headers = vec![
            (String::from("Authorization"), String::from("signature")),
            (String::from("X-Amz-Security-Token"), String::from("token")),
            (
                String::from("Content-Type"),
                String::from("application/json"),
            ),
        ];
        let map = header_map(&headers).expect("valid headers");
        assert!(
            map.get("authorization")
                .is_some_and(|value| value.is_sensitive())
        );
        assert!(
            map.get("x-amz-security-token")
                .is_some_and(|value| value.is_sensitive())
        );
        assert!(
            map.get("content-type")
                .is_some_and(|value| !value.is_sensitive())
        );
        assert!(matches!(
            header_map(&[(String::from("bad name"), String::from("value"))]),
            Err(RuntimeError::Header { name }) if name == "bad name"
        ));
        assert!(matches!(
            header_map(&[(String::from("x-test"), String::from("bad\nvalue"))]),
            Err(RuntimeError::Header { name }) if name == "x-test"
        ));
    }

    #[test]
    fn outbound_validates_url_and_builds_post_request() {
        let counter = Counter;
        let runtime = SagemakerRuntime {
            transport: (),
            credentials: (),
            environment: SagemakerEnvironment::default(),
            clock: aws_smithy_async::time::SharedTimeSource::new(
                aws_smithy_async::time::StaticTimeSource::new(std::time::UNIX_EPOCH),
            ),
            counter: &counter,
            timeout: Duration::from_secs(3),
        };
        let prepared = PreparedCall {
            url: String::from("https://runtime.example/invocations"),
            headers: vec![(
                String::from("Content-Type"),
                String::from("application/json"),
            )],
            body: br#"{"inputs":"hi"}"#.to_vec(),
            body_json: IndexMap::from([(String::from("inputs"), Json::Text(String::from("hi")))]),
            region: String::from("us-west-2"),
        };
        let request = runtime.outbound(&prepared).expect("valid outbound request");
        assert_eq!(request.method, http::Method::POST);
        assert_eq!(request.url.as_str(), "https://runtime.example/invocations");
        assert_eq!(request.deadline, Duration::from_secs(3));
        assert!(matches!(
            runtime.outbound(&PreparedCall {
                url: String::from("not a URL"),
                ..prepared
            }),
            Err(RuntimeError::Endpoint { .. })
        ));
    }

    #[test]
    fn prepared_call_logging_uses_body_object_and_signed_headers() {
        let prepared = PreparedCall {
            url: String::from("https://runtime.example/invocations"),
            headers: vec![(String::from("Authorization"), String::from("secret"))],
            body: Vec::new(),
            body_json: IndexMap::from([(String::from("inputs"), Json::Text(String::from("hi")))]),
            region: String::from("us-west-2"),
        };
        let logged = prepared.additional_args();
        assert_eq!(
            logged.api_base.as_deref(),
            Some("https://runtime.example/invocations")
        );
        assert!(matches!(
            logged.complete_input_dict,
            Some(RawBody::Object(_))
        ));
        assert!(logged.headers.is_some());
        assert!(logged.passthrough.is_empty());
    }

    #[tokio::test]
    async fn proxied_client_transport_reports_unavailable_upstream() {
        let policy = betterlite_config::proxy::ProxyPolicy::resolve(
            &betterlite_config::types::ProxyTrust::default(),
            &std::collections::BTreeMap::new(),
        )
        .expect("default proxy policy");
        let client =
            ProxiedClient::outbound(&std::sync::Arc::new(policy)).expect("outbound client");
        let outbound = || OutboundRequest {
            method: Method::POST,
            url: Url::parse("http://127.0.0.1:9/invocations").expect("test URL"),
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: RequestBody::Empty,
            deadline: Duration::from_millis(200),
        };
        assert!(SagemakerTransport::send(&client, outbound()).await.is_err());
        assert!(
            SagemakerTransport::open_stream(&client, outbound())
                .await
                .is_err()
        );
    }
}
