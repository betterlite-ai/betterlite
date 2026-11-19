use super::*;

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

impl RealtimeRuntime for FailingRuntime {
    type Error = Failing;

    fn connect(
        &self,
        _config: RuntimeConfig,
    ) -> impl Future<Output = Result<Box<dyn RealtimeClient<Error = Failing>>, StreamError<Failing>>>
    + Send {
        let outcome: Result<Box<dyn RealtimeClient<Error = Failing>>, StreamError<Failing>> =
            Err(StreamError {
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
    ) -> impl Future<Output = Result<Option<Credentials>, CredentialError>> + Send {
        ready(Ok(Some(credentials())))
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

fn transport_kind(error: &RealtimeTransportError<Failing, io::Error>) -> &'static str {
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

pub(crate) fn realtime_row(run: &RealtimeRun) -> HelperResult {
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
        model: REALTIME_MODEL,
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
    let result = block_on(realtime_bridge::session(&runtime, &resolver, call))?;
    let mut row = empty_row();
    row.insert("response".to_owned(), realtime_response(result, &scope)?);
    Ok(Value::Object(row))
}

fn realtime_response(
    result: Result<realtime_bridge::Outcome, realtime_bridge::Error>,
    scope: &EventScope,
) -> Result<Value, Box<HelperError>> {
    let committed = matches!(&scope.session_committed, Json::Bool(true));
    match result {
        Ok(_) => Ok(json!({
            "outcome": "ok",
            "error_kind": Value::Null,
            "status_code": Value::Null,
            "committed": committed,
        })),
        Err(realtime_bridge::Error::Session { source }) => {
            let transport = source
                .downcast_ref::<RealtimeTransportError<Failing, io::Error>>()
                .ok_or_else(|| {
                    Box::new(HelperError {
                        class: "ValueError".to_owned(),
                        status: None,
                        error_type: Some(None),
                        param: Some(None),
                        code: Some(None),
                        message: "the realtime session error is not a transport error".to_owned(),
                    })
                })?;
            Ok(json!({
                "outcome": "error",
                "error_kind": transport_kind(transport),
                "status_code": transport.status_code(),
                "committed": committed,
            }))
        }
    }
}
