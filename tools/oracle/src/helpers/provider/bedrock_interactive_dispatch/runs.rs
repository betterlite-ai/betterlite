use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::aws_settings::AuthParams;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_bedrock::passthrough::collector::StreamLogAssembler;
use betterlite_bedrock::passthrough::reply::{NativeReplyRequest, NativeReplyTransform};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::{RealtimeClient, StreamError};
use betterlite_bedrock::realtime_transport::error::{ProviderClass, RealtimeTransportError};
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_bedrock::realtime_transport::runtime::{RealtimeRuntime, RuntimeConfig};
use betterlite_bedrock::realtime_transport::session::RealtimeCall;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_bedrock::vector_stores::request::{SearchOptionalParams, SearchQuery};
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, SessionInput, SuccessWork,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::model_capability_predicates::CapabilityRequest;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_openai::reasoning_models::{DropParams, ReasoningCapabilities};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{ModelInfo, ResponsesCapabilities};
use betterlite_openai::response_params::params::ResponsesParams;
use betterlite_openai::search::provider::SearchQuery as SearchInput;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_provider::bedrock_interactive_dispatch::http::{
    LeafReply, LeafRequest, LeafTransport,
};
use betterlite_provider::bedrock_interactive_dispatch::{
    passthrough, realtime_bridge, responses, search, vector_retrieve,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{body_json, empty_row, error_row, request_row};

pub(crate) mod passthrough_run;
pub(crate) mod realtime_run;
pub(crate) mod responses_run;
pub(crate) mod search_run;
pub(crate) mod vector_run;

const RESPONSES_HOST: &str = "https://bedrock-runtime.us-west-2.amazonaws.com";
const GATEWAY: &str = "https://gateway.example.com/mcp";
const VECTOR_ID: &str = "KB123";
const PASSTHROUGH_MODEL: &str = "anthropic.claude-3";
const REALTIME_MODEL: &str = "amazon.nova-sonic-v1:0";
const ACCESS: &str = "AKIDBETTERLITEFIXTURE";
const SECRET: &str = "fixture/secret/key/+EXAMPLEKEY0000";
const CATALOG: &str = r#"{"openai.gpt-oss-120b": {"supported_endpoints": ["/v1/responses"]}}"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponsesRun {
    model: String,
    #[serde(default)]
    model_cost: ParamMap,
    input: Value,
    #[serde(default)]
    params: Option<Value>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PassthroughRun {
    endpoint: String,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    region: Option<String>,
    status: u16,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    chunks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VectorRun {
    query: String,
    #[serde(default)]
    optional: Option<Value>,
    #[serde(default)]
    extra_body: Option<Value>,
    status: u16,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RealtimeRun {
    pending: String,
}

fn fixture_time() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_767_225_600))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from(ACCESS.to_owned()),
        secret_access_key: SecretString::from(SECRET.to_owned()),
        session_token: None,
    }
}

fn block_on<F: Future>(future: F) -> Result<F::Output, Box<HelperError>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_config_error()?;
    Ok(runtime.block_on(future))
}

fn reply(status: u16, body: &str) -> LeafReply {
    LeafReply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
        chunks: Vec::new(),
    }
}

struct Scripted(LeafReply);

impl LeafTransport for Scripted {
    fn send(
        &self,
        _request: LeafRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LeafReply, TransportError>> + Send + '_>> {
        Box::pin(ready(Ok(self.0.clone())))
    }
}

fn success_row(request: &LeafRequest) -> Map<String, Value> {
    let mut row = empty_row();
    row.insert(
        "request".to_owned(),
        request_row("POST", &request.url, &request.headers, &request.body),
    );
    row
}

fn outcome_row<T, E>(
    outcome: Result<T, E>,
    failure: fn(&E) -> Value,
    success: fn(&T) -> Value,
) -> Value {
    match outcome {
        Ok(value) => success(&value),
        Err(error) => failure(&error),
    }
}
