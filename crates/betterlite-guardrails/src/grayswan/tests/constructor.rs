fn args() -> GraySwanArgs {
    GraySwanArgs {
        name: "safety".to_owned(),
        api_key: Some(SecretString::from("secret")),
        api_base: None,
        on_flagged_action: None,
        violation_threshold: None,
        reasoning_mode: None,
        categories: None,
        policy_id: None,
        streaming_end_of_stream_only: false,
        streaming_sampling_rate: 5,
        fail_open: None,
        guardrail_timeout: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    }
}

fn constructor_env() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("GRAYSWAN_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "GRAYSWAN_API_BASE".to_owned(),
            "https://env.example/".to_owned(),
        ),
        ("GRAYSWAN_VIOLATION_THRESHOLD".to_owned(), "0.7".to_owned()),
        ("GRAYSWAN_REASONING_MODE".to_owned(), "THINKING".to_owned()),
    ])
}

fn empty_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn build(guardrail_args: GraySwanArgs, env: &BTreeMap<String, String>) -> GraySwanGuardrail {
    GraySwanGuardrail::new(guardrail_args, base(), reqwest::Client::default(), env).unwrap()
}

#[test]
fn grayswan_constructor_uses_litellm_defaults() {
    let instance = build(args(), &empty_env());
    let settings = &instance.settings;
    assert_eq!(settings.guardrail_name, "safety");
    assert_eq!(settings.api_base, DEFAULT_API_BASE);
    assert_eq!(settings.on_flagged_action, OnFlaggedAction::Monitor);
    assert!((settings.violation_threshold - 0.5).abs() < f64::EPSILON);
    assert_eq!(settings.reasoning_mode, None);
    assert!(settings.fail_open);
    assert_eq!(settings.guardrail_timeout, Duration::from_secs(30));
    assert_eq!(settings.streaming_sampling_rate, 5);
    assert!(!settings.streaming_end_of_stream_only);
    assert_eq!(
        instance.base.settings.guardrail_name.as_deref(),
        Some("safety")
    );
    assert_eq!(
        instance.base.settings.supported_event_hooks,
        Some(GraySwanGuardrail::get_supported_event_hooks())
    );
}

#[test]
fn grayswan_constructor_reads_the_environment() {
    let settings = build(args(), &constructor_env()).settings;
    assert_eq!(settings.api_key.expose_secret(), "env-key");
    assert_eq!(settings.api_base, "https://env.example");
    assert!((settings.violation_threshold - 0.7).abs() < f64::EPSILON);
    assert_eq!(settings.reasoning_mode, Some(ReasoningMode::Thinking));
}

#[test]
fn grayswan_constructor_prefers_arguments_over_the_environment() {
    let mut guardrail_args = args();
    guardrail_args.api_key = Some(SecretString::from("arg-key"));
    guardrail_args.api_base = Some("https://arg.example/".to_owned());
    guardrail_args.violation_threshold = Some(0.9);
    guardrail_args.reasoning_mode = Some("off".to_owned());
    guardrail_args.on_flagged_action = Some("BLOCK".to_owned());
    guardrail_args.fail_open = Some(false);
    guardrail_args.guardrail_timeout = Some(2.5);
    guardrail_args.categories = Some(BTreeMap::from([("hate".to_owned(), "block".to_owned())]));
    guardrail_args.policy_id = Some("policy".to_owned());
    guardrail_args.streaming_end_of_stream_only = true;
    let settings = build(guardrail_args, &constructor_env()).settings;
    assert_eq!(settings.api_key.expose_secret(), "arg-key");
    assert_eq!(settings.api_base, "https://arg.example");
    assert!((settings.violation_threshold - 0.9).abs() < f64::EPSILON);
    assert_eq!(settings.reasoning_mode, Some(ReasoningMode::Off));
    assert_eq!(settings.on_flagged_action, OnFlaggedAction::Block);
    assert!(!settings.fail_open);
    assert_eq!(settings.guardrail_timeout, Duration::from_millis(2500));
    assert_eq!(
        settings.categories,
        BTreeMap::from([("hate".to_owned(), "block".to_owned())])
    );
    assert_eq!(settings.policy_id, "policy");
    assert!(settings.streaming_end_of_stream_only);
}

#[test]
fn grayswan_constructor_falls_back_to_the_default_base_for_an_empty_environment() {
    let env = BTreeMap::from([("GRAYSWAN_API_BASE".to_owned(), String::new())]);
    assert_eq!(build(args(), &env).settings.api_base, DEFAULT_API_BASE);
}

#[test]
fn grayswan_constructor_falls_back_to_the_environment_for_an_empty_base_argument() {
    let mut guardrail_args = args();
    guardrail_args.api_base = Some(String::new());
    assert_eq!(
        build(guardrail_args, &constructor_env()).settings.api_base,
        "https://env.example"
    );
}

#[test]
fn grayswan_constructor_falls_back_to_the_environment_for_an_empty_key_argument() {
    let mut guardrail_args = args();
    guardrail_args.api_key = Some(SecretString::from(""));
    let settings = build(guardrail_args, &constructor_env()).settings;
    assert_eq!(settings.api_key.expose_secret(), "env-key");
}

#[test]
fn grayswan_constructor_rejects_a_missing_key() {
    let mut guardrail_args = args();
    guardrail_args.api_key = None;
    let error = GraySwanGuardrail::new(
        guardrail_args,
        base(),
        reqwest::Client::default(),
        &empty_env(),
    )
    .expect_err("a missing key is an error");
    assert!(matches!(
        error,
        GraySwanGuardrailSetupError::MissingSecrets(_)
    ));
    assert_eq!(
        error.to_string(),
        "Gray Swan API key missing. Set `GRAYSWAN_API_KEY` or pass `api_key`."
    );
}

#[test]
fn grayswan_constructor_rejects_an_empty_key_in_the_environment() {
    let mut guardrail_args = args();
    guardrail_args.api_key = None;
    let env = BTreeMap::from([("GRAYSWAN_API_KEY".to_owned(), String::new())]);
    let error = GraySwanGuardrail::new(guardrail_args, base(), reqwest::Client::default(), &env)
        .expect_err("an empty key is a missing key");
    assert!(matches!(
        error,
        GraySwanGuardrailSetupError::MissingSecrets(_)
    ));
}

#[rstest]
#[case::block("BLOCK", OnFlaggedAction::Block)]
#[case::monitor("Monitor", OnFlaggedAction::Monitor)]
#[case::passthrough("PASSTHROUGH", OnFlaggedAction::Passthrough)]
#[case::empty("", OnFlaggedAction::Monitor)]
#[case::unsupported("bogus", OnFlaggedAction::Monitor)]
fn grayswan_constructor_normalizes_the_flagged_action(
    #[case] action: &str,
    #[case] expected: OnFlaggedAction,
) {
    let mut guardrail_args = args();
    guardrail_args.on_flagged_action = Some(action.to_owned());
    assert_eq!(
        build(guardrail_args, &empty_env())
            .settings
            .on_flagged_action,
        expected
    );
}

#[test]
fn grayswan_constructor_does_not_warn_for_the_monitor_action() {
    let captured = Captured::default();
    let mut guardrail_args = args();
    guardrail_args.on_flagged_action = Some("monitor".to_owned());
    let instance = with_default(subscriber(captured.clone(), tracing::Level::WARN), || {
        build(guardrail_args, &empty_env())
    });
    assert_eq!(
        instance.settings.on_flagged_action,
        OnFlaggedAction::Monitor
    );
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(!output.contains("unsupported on_flagged_action"));
}

#[test]
fn grayswan_constructor_warns_for_an_unsupported_flagged_action() {
    let captured = Captured::default();
    let mut guardrail_args = args();
    guardrail_args.on_flagged_action = Some("bogus".to_owned());
    let instance = with_default(subscriber(captured.clone(), tracing::Level::WARN), || {
        build(guardrail_args, &empty_env())
    });
    assert_eq!(
        instance.settings.on_flagged_action,
        OnFlaggedAction::Monitor
    );
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("unsupported on_flagged_action"));
}

#[test]
fn grayswan_constructor_rejects_a_negative_timeout() {
    let mut guardrail_args = args();
    guardrail_args.guardrail_timeout = Some(-1.0);
    let error = GraySwanGuardrail::new(
        guardrail_args,
        base(),
        reqwest::Client::default(),
        &empty_env(),
    )
    .expect_err("a negative timeout is an error");
    assert!(matches!(error, GraySwanGuardrailSetupError::Timeout { .. }));
}

#[test]
fn grayswan_constructor_rejects_a_negative_sampling_rate() {
    let mut guardrail_args = args();
    guardrail_args.streaming_sampling_rate = -1;
    let error = GraySwanGuardrail::new(
        guardrail_args,
        base(),
        reqwest::Client::default(),
        &empty_env(),
    )
    .expect_err("a negative sampling rate is an error");
    assert!(matches!(
        error,
        GraySwanGuardrailSetupError::SamplingRate { .. }
    ));
}

#[test]
fn grayswan_constructor_keeps_a_configured_base_name() {
    let settings = GuardrailSettings {
        guardrail_name: Some("given".to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    };
    let instance = GraySwanGuardrail::new(
        args(),
        GuardrailBase::new(settings, true).unwrap(),
        reqwest::Client::default(),
        &empty_env(),
    )
    .unwrap();
    assert_eq!(
        instance.base.settings.guardrail_name.as_deref(),
        Some("given")
    );
}

#[test]
fn grayswan_constructor_logs_the_streaming_configuration() {
    let captured = Captured::default();
    let settings = with_default(subscriber(captured.clone(), tracing::Level::DEBUG), || {
        build(args(), &empty_env()).settings
    });
    assert_eq!(settings.streaming_sampling_rate, 5);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains(
        "GraySwan __init__: streaming_end_of_stream_only=false, streaming_sampling_rate=5"
    ));
}

fn requested_data() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([(
        "guardrails".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Text("safety".to_owned())]),
    )])
}

#[test]
fn grayswan_should_run_follows_the_event_hook() {
    let mut instance = guardrail(OnFlaggedAction::Monitor);
    instance.base.settings.event_hook = Some(LitellmParamsMode::One("post_call".to_owned()));
    let data = requested_data();
    let suppressed = BTreeSet::new();
    assert!(
        !instance
            .should_run_guardrail(&data, GuardrailEventHooks::PreCall, &suppressed)
            .unwrap()
    );
    assert!(
        instance
            .should_run_guardrail(&data, GuardrailEventHooks::PostCall, &suppressed)
            .unwrap()
    );
}

#[test]
fn grayswan_should_run_requires_a_requested_guardrail_when_it_is_not_default_on() {
    let mut instance = guardrail(OnFlaggedAction::Monitor);
    instance.base.settings.event_hook = Some(LitellmParamsMode::One("pre_call".to_owned()));
    let suppressed = BTreeSet::new();
    assert!(
        !instance
            .should_run_guardrail(&BTreeMap::new(), GuardrailEventHooks::PreCall, &suppressed)
            .unwrap()
    );
}

#[test]
fn grayswan_should_run_respects_suppression() {
    let mut instance = guardrail(OnFlaggedAction::Monitor);
    instance.base.settings.default_on = true;
    let suppressed = BTreeSet::from(["safety".to_owned()]);
    assert!(
        !instance
            .should_run_guardrail(&BTreeMap::new(), GuardrailEventHooks::PreCall, &suppressed)
            .unwrap()
    );
    assert!(
        instance
            .should_run_guardrail(
                &BTreeMap::new(),
                GuardrailEventHooks::PreCall,
                &BTreeSet::new()
            )
            .unwrap()
    );
}

#[test]
fn grayswan_should_run_rejects_a_non_list_requested_guardrails() {
    let mut instance = guardrail(OnFlaggedAction::Monitor);
    instance.base.settings.event_hook = Some(LitellmParamsMode::One("pre_call".to_owned()));
    let data = BTreeMap::from([(
        "guardrails".to_owned(),
        GuardrailValue::Text("safety".to_owned()),
    )]);
    assert!(
        instance
            .should_run_guardrail(&data, GuardrailEventHooks::PreCall, &BTreeSet::new())
            .is_err()
    );
}

#[test]
fn grayswan_should_run_logs_the_debug_fields() {
    let captured = Captured::default();
    let mut instance = guardrail(OnFlaggedAction::Monitor);
    instance.base.settings.default_on = true;
    instance.base.settings.event_hook = Some(LitellmParamsMode::One("pre_call".to_owned()));
    let selected = with_default(subscriber(captured.clone(), tracing::Level::DEBUG), || {
        instance
            .should_run_guardrail(
                &BTreeMap::new(),
                GuardrailEventHooks::PreCall,
                &BTreeSet::new(),
            )
            .unwrap()
    });
    assert!(selected);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("GraySwan DEBUG: should_run_guardrail"));
    assert!(output.contains("event_type"));
    assert!(output.contains("pre_call"));
    assert!(output.contains("event_hook"));
    assert!(output.contains("has_apply_guardrail"));
    assert!(output.contains("GraySwanGuardrail"));
}

fn parse_model(json: &str) -> GraySwanGuardrailConfigModel {
    serde_json::from_str(json).expect("config model json")
}

fn optional_params(model: &GraySwanGuardrailConfigModel) -> &GraySwanConfigOptionalParams {
    model.optional_params.as_ref().expect("optional params")
}

#[test]
fn grayswan_config_model_defaults_match_litellm() {
    let model = parse_model(r#"{"optional_params": {}}"#);
    let params = optional_params(&model);
    assert_eq!(params.on_flagged_action.as_deref(), Some("passthrough"));
    assert_eq!(params.violation_threshold, Some(0.5));
    assert_eq!(params.fail_open, Some(true));
    assert_eq!(params.guardrail_timeout, Some(30.0));
    assert_eq!(params.reasoning_mode, None);
    assert_eq!(params.policy_id, None);
    assert_eq!(params.categories, None);
    assert!(model.api_key.is_none());
    assert!(model.api_base.is_none());
}

#[test]
fn grayswan_config_model_keeps_explicit_values() {
    let model = parse_model(
        r#"{"api_key": "model-key", "api_base": "https://model.example",
            "optional_params": {"on_flagged_action": "block", "violation_threshold": 0.8,
            "reasoning_mode": "hybrid", "policy_id": "policy", "fail_open": false,
            "guardrail_timeout": 5.0}}"#,
    );
    let params = optional_params(&model);
    assert_eq!(
        model
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("model-key".to_owned())
    );
    assert_eq!(model.api_base.as_deref(), Some("https://model.example"));
    assert_eq!(params.on_flagged_action.as_deref(), Some("block"));
    assert_eq!(params.violation_threshold, Some(0.8));
    assert_eq!(params.reasoning_mode.as_deref(), Some("hybrid"));
    assert_eq!(params.policy_id.as_deref(), Some("policy"));
    assert_eq!(params.fail_open, Some(false));
    assert_eq!(params.guardrail_timeout, Some(5.0));
}

#[test]
fn grayswan_config_model_ui_name_is_the_litellm_name() {
    assert_eq!(
        GraySwanGuardrailConfigModel::ui_friendly_name(),
        "Gray Swan Guardrail"
    );
}

#[test]
fn grayswan_config_model_default_action_differs_from_the_constructor_default() {
    let model = parse_model(r#"{"optional_params": {}}"#);
    let configured_action = optional_params(&model).on_flagged_action.as_deref();
    let constructed_action = build(args(), &empty_env()).settings.on_flagged_action;
    assert_eq!(configured_action, Some("passthrough"));
    assert_eq!(constructed_action, OnFlaggedAction::Monitor);
}
