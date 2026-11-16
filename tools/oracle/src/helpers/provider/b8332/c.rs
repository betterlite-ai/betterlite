fn failure(error: HubDispatchError) -> Box<HelperError> {
    let (class, status) = match &error {
        HubDispatchError::Replicate(_) => ("ReplicateError", Some(400)),
        HubDispatchError::PredibaseSetup(_) => ("ValueError", None),
        HubDispatchError::Embedding(_) => ("HuggingFaceError", Some(500)),
        _ => ("HubDispatchError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_failure("RuntimeError", error))
}

fn call_context<'source>(
    case: &'source Case,
    messages: &'source mut Vec<Message>,
    options: &'source mut Object,
    key: Option<&'source SecretString>,
) -> ChatContext<'source> {
    ChatContext {
        model: &case.model,
        messages,
        optional_params: options,
        api_key: key,
        api_base: case.api_base.as_deref(),
        headers: None,
        custom_prompt_dict: None,
        acompletion: false,
    }
}

fn chat(case: &Case, runtime: &Runtime) -> HelperResult {
    let fixture = Fixture::new(&case.replies, &case.polls);
    let globals = globals(None);
    let deps = fixture.deps(&globals, &case.env);
    let mut messages = case.messages.clone();
    let mut options = options(case);
    let key = case.api_key.clone().map(SecretString::from);
    let mut call = call_context(case, &mut messages, &mut options, key.as_ref());
    let prepared = runtime
        .block_on(huggingface::chat_request(&mut call, &deps))
        .map_err(failure)?;
    let (kind, http) = match &prepared {
        PreparedRequest::Stream(chat) => ("stream", &chat.http),
        PreparedRequest::Chat(chat) => ("chat", &chat.http),
        _ => return Err(failure(HubDispatchError::BodyNotObject)),
    };
    Ok(json!({
        "kind": kind,
        "requests": [{
            "method": http.method.as_str(),
            "url": http.url.as_str(),
            "headers": headers_value(&http.headers),
            "body": body_value(&http.body),
        }],
    }))
}

fn embedding(case: &Case, runtime: &Runtime) -> HelperResult {
    let fixture = Fixture::new(&case.replies, &case.polls);
    let globals = globals(None);
    let deps = fixture.deps(&globals, &case.env);
    let options = options(case);
    let key = case.api_key.clone().map(SecretString::from);
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: &case.model,
        input: &case.input,
        optional_params: &options,
        api_key: key.as_ref(),
        api_base: case.api_base.as_deref(),
        headers: &headers,
    };
    let (mut state, redactor, sink) = log_state()?;
    let env = pre_post_env(&redactor, &sink);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    runtime
        .block_on(embed(&call, &deps, &mut log))
        .map_err(failure)?;
    Ok(json!({"requests": posted_requests(&fixture.post)}))
}

fn rerank(case: &Case) -> HelperResult {
    let globals = globals(None);
    let key = case.api_key.clone().map(SecretString::from);
    let config =
        huggingface_rerank_config(key.as_ref(), case.api_base.as_deref(), &globals, &case.env)
            .map_err(failure)?;
    let headers = Headers::new();
    let resolved = config
        .validate_environment(EnvironmentRequest {
            headers: &headers,
            model: &case.model,
            api_key: key.as_ref(),
            optional_params: None,
            litellm_params: None,
        })
        .map_err(|error| failure(HubDispatchError::Rerank(error)))?;
    let url = config.get_complete_url(case.api_base.as_deref(), &case.model, None);
    Ok(json!({"url": url, "headers": posted_headers_value(&resolved)}))
}

fn replicate(case: &Case, runtime: &Runtime) -> HelperResult {
    let fixture = Fixture::new(&case.replies, &case.polls);
    let globals = globals(None);
    let deps = fixture.deps(&globals, &case.env);
    let mut messages = case.messages.clone();
    let mut options = options(case);
    let key = case.api_key.clone().map(SecretString::from);
    let mut call = call_context(case, &mut messages, &mut options, key.as_ref());
    let (mut state, redactor, sink) = log_state()?;
    let env = pre_post_env(&redactor, &sink);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    runtime
        .block_on(replicate::complete(&mut call, &deps, &mut log))
        .map_err(failure)?;
    let requests = fixture
        .transport
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut output = Vec::with_capacity(requests.len());
    for request in requests.iter() {
        output.push(json!({
            "method": request.method.as_str(),
            "url": request.url,
            "headers": headers_value(&request.headers),
            "body": request.body.as_ref().map_or(Value::Null, |bytes| body_value(bytes)),
        }));
    }
    Ok(json!({"requests": output}))
}

fn predibase(case: &Case, runtime: &Runtime) -> HelperResult {
    let fixture = Fixture::new(&case.replies, &case.polls);
    let globals = globals(None);
    let deps = fixture.deps(&globals, &case.env);
    let mut messages = case.messages.clone();
    let mut options = options(case);
    let key = case.api_key.clone().map(SecretString::from);
    let mut call = call_context(case, &mut messages, &mut options, key.as_ref());
    let (mut state, redactor, sink) = log_state()?;
    let env = pre_post_env(&redactor, &sink);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    runtime
        .block_on(predibase::complete(&mut call, &deps, &mut log))
        .map_err(failure)?;
    Ok(json!({"requests": posted_requests(&fixture.post)}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Case;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = runtime()?;
        match input.op {
            Operation::HuggingfaceChat => chat(input, &runtime),
            Operation::HuggingfaceEmbedding => embedding(input, &runtime),
            Operation::HuggingfaceRerank => rerank(input),
            Operation::Replicate => replicate(input, &runtime),
            Operation::Predibase => predibase(input, &runtime),
        }
    }
}
