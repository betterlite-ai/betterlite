struct Fixture {
    hub: NoHub,
    post: ScriptedPost,
    transport: Arc<ScriptedTransport>,
    sleeper: Arc<Sleeper>,
    logger: Logger,
    prompts: PromptFactory,
    replicate_counter: ReplicateCounting,
    predibase_counter: PlainCounting,
    encoder: LengthEncoder,
    clock: FixedClock,
}

impl Fixture {
    fn new(replies: &[Reply], polls: &[Reply]) -> Self {
        let post_replies = replies
            .iter()
            .map(|reply| UpstreamResponse {
                status: reply.status,
                headers: HeaderMap::new(),
                body: reply.body.as_bytes().to_vec(),
            })
            .collect();
        let transport_replies = polls
            .iter()
            .map(|reply| ReplicateResponse {
                status: reply.status,
                headers: HeaderMap::new(),
                body: reply.body.as_bytes().to_vec(),
            })
            .collect();
        Self {
            hub: NoHub,
            post: ScriptedPost {
                replies: Mutex::new(post_replies),
                requests: Mutex::new(Vec::new()),
            },
            transport: Arc::new(ScriptedTransport {
                replies: Mutex::new(transport_replies),
                requests: Mutex::new(Vec::new()),
            }),
            sleeper: Arc::new(Sleeper),
            logger: Logger,
            prompts: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            replicate_counter: ReplicateCounting,
            predibase_counter: PlainCounting,
            encoder: LengthEncoder,
            clock: FixedClock,
        }
    }

    fn deps<'deps>(
        &'deps self,
        globals: &'deps Globals<'deps>,
        env: &'deps BTreeMap<String, String>,
    ) -> DispatchDeps<'deps> {
        DispatchDeps {
            globals,
            environment: env,
            hub: &self.hub,
            post: &self.post,
            encoder: &self.encoder,
            replicate: ReplicateServices {
                transport: Arc::clone(&self.transport),
                sleeper: Arc::clone(&self.sleeper),
                logger: &self.logger,
            },
            prompts: &self.prompts,
            replicate_counter: &self.replicate_counter,
            predibase_counter: &self.predibase_counter,
            clock: &self.clock,
            default_max_tokens: 4096,
            now: self.clock.now(),
            timeout: Duration::from_secs(5),
        }
    }
}

fn globals<'source>(api_base: Option<&'source str>) -> Globals<'source> {
    Globals {
        api_key: None,
        huggingface_key: None,
        replicate_key: None,
        predibase_key: None,
        predibase_tenant_id: None,
        api_base,
        headers: None,
        custom_prompt_dict: None,
    }
}

fn options(case: &Case) -> Object {
    let mut options = case.optional_params.clone();
    if let Some(input_type) = &case.input_type {
        options.insert(
            String::from("input_type"),
            PromptJson::Text(input_type.clone()),
        );
    }
    if let Some(tenant_id) = &case.tenant_id {
        options.insert(
            String::from("tenant_id"),
            PromptJson::Text(tenant_id.clone()),
        );
    }
    if case.stream {
        options.insert(String::from("stream"), PromptJson::Bool(true));
    }
    options
}

fn body_value(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn header_text(name: &str) -> bool {
    name == "authorization" || name == "content-type" || name == "accept" || name.starts_with("x-")
}

fn headers_value(headers: &HeaderMap) -> Value {
    let mut found = Map::new();
    for (name, value) in headers {
        if header_text(name.as_str()) {
            if let Ok(text) = value.to_str() {
                found.insert(name.as_str().to_owned(), Value::String(text.to_owned()));
            }
        }
    }
    Value::Object(found)
}

fn posted_headers_value(headers: &Headers) -> Value {
    let mut found = Map::new();
    for (name, value) in headers {
        if header_text(name) {
            found.insert(name.to_lowercase(), Value::String(value.clone()));
        }
    }
    Value::Object(found)
}

fn posted_requests(post: &ScriptedPost) -> Value {
    let requests = post.requests.lock().unwrap_or_else(PoisonError::into_inner);
    let mut output = Vec::with_capacity(requests.len());
    for (url, headers, body) in requests.iter() {
        output.push(json!({
            "method": "POST",
            "url": url,
            "headers": posted_headers_value(headers),
            "body": body_value(body.as_bytes()),
        }));
    }
    Value::Array(output)
}

fn log_state() -> Result<(LoggingState, Redactor, Discard), Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_failure("Redactor", error))?;
    let mut context = CorrelationContext::default();
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT)
            .map_err(|error| helper_failure("LoggingInit", error))?,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| helper_failure("LoggingState", error))?;
    Ok((state, redactor, Discard))
}

fn pre_post_env<'env>(redactor: &'env Redactor, sink: &'env Discard) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").unwrap_or_default(),
    }
}
