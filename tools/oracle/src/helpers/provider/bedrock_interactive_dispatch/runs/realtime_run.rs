use std::future::{Future, ready};
use std::io;

use betterlite_bedrock::agentcore_stream::IdSource;
use betterlite_bedrock::aws_settings::AuthParams;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::realtime_events::client::{ClientError, RealtimeClientWebSocket};
use betterlite_bedrock::realtime_events::scope::EventScope;
use betterlite_bedrock::realtime_events::stream::StreamError;
use betterlite_bedrock::realtime_transport::error::{ProviderClass, RealtimeTransportError};
use betterlite_bedrock::realtime_transport::prepare::RealtimeCredentials;
use betterlite_bedrock::realtime_transport::runtime::{
    RealtimeClient, RealtimeRuntime, RuntimeConfig,
};
use betterlite_bedrock::realtime_transport::session::RealtimeCall;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_config::api::realtime_session_state::LoggedEventTypes;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::realtime_session_state::logging::{
    RealtimeLogging, SessionInput, SuccessWork,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_provider::bedrock_interactive_dispatch::realtime_bridge;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::{RealtimeRun, shared};

#[derive(Debug, thiserror::Error)]
#[error("open failed")]
struct Failing;

impl ProviderClass for Failing {
    fn provider_class_name(&self) -> Option<&str> {
        Some("WebSocketError")
    }

    fn provider_message(&self) -> Option<&str> {
        Some("open failed")
    }
}

struct FailingRuntime;

type Opened = Result<Box<dyn RealtimeClient<Error = Failing>>, StreamError<Failing>>;

impl RealtimeRuntime for FailingRuntime {
    type Error = Failing;

    fn connect(&self, _config: RuntimeConfig) -> impl Future<Output = Opened> + Send {
        let outcome: Opened = Err(StreamError {
            message: "the websocket could not open".to_owned(),
            source: Some(Failing),
        });
        ready(outcome)
    }
}

struct Resolver;

impl RealtimeCredentials for Resolver {
    fn resolve(
        &self,
        _settings: &AwsSettings,
        _region: &str,
    ) -> impl Future<Output = Result<Option<shared::Credentials>, CredentialError>> + Send {
        ready(Ok(Some(shared::credentials())))
    }
}

struct Socket;

#[async_trait::async_trait]
impl RealtimeClientWebSocket for Socket {
    type Error = io::Error;

    async fn receive_text(&self) -> Result<String, ClientError<io::Error>> {
        Err(ClientError {
            message: "the client left".to_owned(),
            source: None,
        })
    }

    async fn send_text(&self, _data: String) -> Result<(), ClientError<io::Error>> {
        Ok(())
    }

    async fn close(&self, _code: u16, _reason: Option<String>) {}
}

struct Silent;

impl RealtimeLogging for Silent {
    fn pre_call(&self, _input: &SessionInput<'_>, _api_key: &str) {}

    fn record_model_call_detail(&self, _key: &str, _value: Json) {}

    fn dispatch_success_handlers(&self, _messages: &[IndexMap<String, Json>]) -> SuccessWork {
        Box::pin(async {})
    }
}

struct NullIds;

impl IdSource for NullIds {
    fn next_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
}

const fn transport_kind(error: &RealtimeTransportError<Failing, io::Error>) -> &'static str {
    match error {
        RealtimeTransportError::Committed(_) => "committed",
        RealtimeTransportError::Endpoint(_) => "endpoint",
        RealtimeTransportError::Credential(_) => "credential",
        RealtimeTransportError::NoCredentials { .. } => "no_credentials",
        RealtimeTransportError::Client(_) => "client",
        RealtimeTransportError::SessionCreated { .. } => "provider_unchanged",
        RealtimeTransportError::Stream(_) => "stream",
        RealtimeTransportError::Completion(_) => "completion",
        RealtimeTransportError::Logging { .. } => "logging",
        RealtimeTransportError::Provider(_) => "provider_bedrock",
    }
}

pub(crate) fn realtime_row(run: &RealtimeRun) -> shared::HelperResult {
    let runtime = FailingRuntime;
    let resolver = Resolver;
    let socket = Socket;
    let logging = Silent;
    let ids = NullIds;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
    let logged_types = LoggedEventTypes::Default;
    let mut scope = EventScope {
        pending_session_update: Json::Text(run.pending.clone()),
        session_committed: Json::Null,
        committed_failure: Json::Null,
    };
    let call = RealtimeCall {
        model: shared::REALTIME_MODEL,
        client: &socket,
        logging: &logging,
        logging_worker: None,
        api_base: None,
        aws_bedrock_runtime_endpoint: None,
        aws_region_name: Some("us-east-1"),
        auth: AuthParams::default(),
        region_inputs: RegionInputs::default(),
        scope: &mut scope,
        ids: &ids,
        redactor: &redactor,
        logged_types: &logged_types,
        trace_id: "trace-interactive",
    };
    let result = shared::block_on(realtime_bridge::session(&runtime, &resolver, call))?;
    let mut row = shared::empty_row();
    row.insert("response".to_owned(), realtime_response(result, &scope));
    Ok(shared::Value::Object(row))
}

fn realtime_response(
    result: Result<
        realtime_bridge::Outcome,
        realtime_bridge::Error<RealtimeTransportError<Failing, io::Error>>,
    >,
    scope: &EventScope,
) -> shared::Value {
    let committed = matches!(&scope.session_committed, Json::Bool(true));
    match result {
        Ok(_) => shared::json!({
            "outcome": "ok",
            "error_kind": shared::Value::Null,
            "status_code": shared::Value::Null,
            "committed": committed,
        }),
        Err(realtime_bridge::Error::Session { source }) => shared::json!({
            "outcome": "error",
            "error_kind": transport_kind(&source),
            "status_code": source.status_code(),
            "committed": committed,
        }),
    }
}
