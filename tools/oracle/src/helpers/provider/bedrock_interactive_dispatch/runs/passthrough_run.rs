use super::*;

struct Selector;

impl ChatConfigSelection for Selector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(Some(ChatConfigTag::Anthropic))
    }
}

struct Transform {
    result: Json,
}

impl NativeReplyTransform for Transform {
    fn transform(&self, _request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
        Ok(self.result.clone())
    }
}

struct Assembler;

impl StreamLogAssembler for Assembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        (!chunks.is_empty()).then(|| json!({"id": "assembled"}))
    }
}

struct Logging;

impl PassthroughLogging for Logging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

struct PassthroughServices {
    selector: Selector,
    transform: Transform,
    assembler: Assembler,
}

impl PassthroughServices {
    fn plan<'call>(&'call self, run: &'call PassthroughRun) -> BedrockPassthroughPlan<'call> {
        BedrockPassthroughPlan {
            environment: PassthroughEnvironment {
                region_name: Some(run.region.clone().unwrap_or_else(|| "us-east-1".to_owned())),
                region: None,
                session_region: None,
                runtime_endpoint: None,
            },
            identity: PassthroughIdentity {
                credentials: Some(credentials()),
                bearer_token: None,
                now: fixture_time(),
            },
            selector: &self.selector,
            transform: &self.transform,
            assembler: &self.assembler,
            clock: CollectorClock {
                next_id: || uuid::Uuid::from_u128(7),
                now: || 1_767_225_600,
                instant: || StreamInstant {
                    seconds: 1_767_225_600,
                    timestamp: 1_767_225_600.0,
                },
            },
        }
    }
}

fn passthrough_url_request<'call>(
    params: &'call Object,
    endpoint: &'call str,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: PassthroughTarget {
            model: PASSTHROUGH_MODEL,
            custom_llm_provider: "bedrock",
            endpoint,
        },
        request_query_params: None,
        litellm_params: params,
    }
}

fn passthrough_transport(run: &PassthroughRun) -> Result<Scripted, Box<HelperError>> {
    let mut scripted = reply(run.status, &run.reply);
    for chunk in &run.chunks {
        scripted.chunks.push(hex::decode(chunk).or_value_error()?);
    }
    Ok(Scripted(scripted))
}

pub(crate) fn passthrough_row(run: &PassthroughRun) -> HelperResult {
    let services = PassthroughServices {
        selector: Selector,
        transform: Transform { result: Json::Null },
        assembler: Assembler,
    };
    let plan = services.plan(run);
    let params = Object::new();
    let url_request = passthrough_url_request(&params, &run.endpoint);
    let Ok((url, endpoint_url)) = plan.get_complete_url(&url_request) else {
        return Ok(error_row("leaf", None));
    };
    let signed_base = url.to_string();
    let data = run
        .data
        .clone()
        .map(serde_json::from_value::<Object>)
        .transpose()?;
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: data.as_ref(),
        api_base: &signed_base,
        model: Some(PASSTHROUGH_MODEL),
    };
    let mut logging = Logging;
    let operation = passthrough::Operation {
        plan,
        url_request,
        signing,
        headers: Headers::new(),
        logging: &mut logging,
    };
    let transport = passthrough_transport(run)?;
    match block_on(operation.run(&transport))? {
        Ok(outcome) => Ok(passthrough_success(&outcome, &endpoint_url)),
        Err(error) => Ok(passthrough_failure(&error)),
    }
}

fn passthrough_success(outcome: &passthrough::Outcome, endpoint_url: &str) -> Value {
    let mut row = success_row(&outcome.request);
    row.insert("endpoint_url".to_owned(), json!(endpoint_url));
    row.insert("streaming".to_owned(), json!(outcome.streaming));
    row.insert("status".to_owned(), json!(outcome.status));
    row.insert("logged".to_owned(), json!(outcome.logged.is_some()));
    Value::Object(row)
}

fn passthrough_failure(error: &passthrough::Error) -> Value {
    match error {
        passthrough::Error::Leaf(_) => error_row("leaf", None),
        passthrough::Error::Transport(_) => error_row("transport", None),
        passthrough::Error::Collector { .. } => error_row("collector", None),
    }
}
