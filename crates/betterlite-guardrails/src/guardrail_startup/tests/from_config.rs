fn settings(guardrails: Vec<Guardrail>) -> Settings {
    Settings {
        guardrails,
        rejected: Vec::new(),
    }
}

#[tokio::test]
async fn from_config_starts_the_guardrails_in_file_order_as_config() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let file = settings(vec![
        guardrail(Some("one"), "first", BEDROCK_PRE),
        guardrail(Some("two"), "second", BEDROCK_POST),
    ]);
    let routed = start_from_config(&runtime, &file).await.unwrap();
    let ids: Vec<Option<&str>> = routed.iter().map(|entry| entry.id.as_deref()).collect();
    assert_eq!(ids, [Some("one"), Some("two")]);
    let names: Vec<&str> = routed
        .iter()
        .map(|entry| entry.guardrail_name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
    let held = runtime.handler().read().await;
    assert_eq!(held_ids(&held), ["one", "two"]);
    assert_eq!(
        held.get_source("one"),
        Some(GuardrailDefinitionLocation::Config)
    );
    assert_eq!(
        held.get_source("two"),
        Some(GuardrailDefinitionLocation::Config)
    );
}

#[tokio::test]
async fn from_config_returns_the_router_entries_after_the_env_replacement() {
    let env = BTreeMap::from([
        ("GUARD_KEY".to_owned(), "resolved-secret".to_owned()),
        ("GUARD_BASE".to_owned(), "https://guard.example".to_owned()),
    ]);
    let (runtime, _reads) = runtime_with(handler_with(env), Vec::new());
    let params = r#"{"guardrail":"bedrock","mode":"pre_call","api_key":"os.environ/GUARD_KEY","api_base":"os.environ/GUARD_BASE"}"#;
    let file = settings(vec![guardrail(Some("one"), "first", params)]);
    let routed = start_from_config(&runtime, &file).await.unwrap();
    let entry = routed.first().unwrap();
    assert_eq!(
        entry.api_key.as_ref().unwrap().expose_secret(),
        "resolved-secret"
    );
    assert_eq!(entry.api_base.as_deref(), Some("https://guard.example"));
    assert!(!format!("{entry:?}").contains("resolved-secret"));
}

#[rstest]
#[case::unsupported_kind(UNSUPPORTED)]
#[case::missing_kind(r#"{"guardrail":"","mode":"pre_call"}"#)]
#[case::mode_the_guardrail_does_not_support(r#"{"guardrail":"bedrock","mode":"logging_only"}"#)]
#[tokio::test]
async fn from_config_logs_and_skips_a_bad_entry_then_continues(#[case] bad: &str) {
    let (captured, _guard) = capture();
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let file = settings(vec![
        guardrail(Some("one"), "first", BEDROCK_PRE),
        guardrail(Some("bad"), "broken", bad),
        guardrail(Some("three"), "third", BEDROCK_PRE),
    ]);
    let routed = start_from_config(&runtime, &file).await.unwrap();
    assert_eq!(routed.len(), 2);
    assert_eq!(runtime_ids(&runtime).await, ["one", "three"]);
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.contains("Skipping guardrail 'broken': invalid configuration, proxy is starting WITHOUT this guardrail: "),
        "{text}"
    );
}

#[tokio::test]
async fn from_config_logs_the_unsupported_kind_in_the_skip_line() {
    let (captured, _guard) = capture();
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let file = settings(vec![guardrail(Some("bad"), "broken", UNSUPPORTED)]);
    assert!(start_from_config(&runtime, &file).await.unwrap().is_empty());
    assert!(logged(&captured).contains("Unsupported guardrail: no-such-guardrail"));
}

#[tokio::test]
async fn from_config_logs_every_rejected_entry_and_still_starts_the_rest() {
    let (captured, _guard) = capture();
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let mut file = settings(vec![guardrail(Some("one"), "first", BEDROCK_PRE)]);
    file.rejected = vec![
        RejectedGuardrail {
            guardrail_name: "bad-one".to_owned(),
            reason: "mode is required".to_owned(),
        },
        RejectedGuardrail {
            guardrail_name: "bad-two".to_owned(),
            reason: "guardrail is required".to_owned(),
        },
    ];
    let routed = start_from_config(&runtime, &file).await.unwrap();
    assert_eq!(routed.len(), 1);
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.contains("Skipping guardrail 'bad-one': invalid configuration, proxy is starting WITHOUT this guardrail: mode is required"),
        "{text}"
    );
    assert!(
        text.contains("Skipping guardrail 'bad-two': invalid configuration, proxy is starting WITHOUT this guardrail: guardrail is required"),
        "{text}"
    );
}

#[tokio::test]
async fn from_config_stops_startup_on_the_hide_secrets_enterprise_error() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let file = settings(vec![
        guardrail(Some("one"), "first", BEDROCK_PRE),
        guardrail(Some("secrets"), "hide", HIDE_SECRETS),
        guardrail(Some("three"), "third", BEDROCK_PRE),
    ]);
    let failed = start_from_config(&runtime, &file).await.unwrap_err();
    assert!(matches!(
        &failed,
        StartupError::Guardrail { guardrail_name, source: RegistryError::Enterprise(_) }
            if guardrail_name == "hide"
    ));
    assert_eq!(runtime_ids(&runtime).await, ["one"]);
}

#[tokio::test]
async fn from_config_with_nothing_to_start_returns_an_empty_list() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    assert!(
        start_from_config(&runtime, &settings(Vec::new()))
            .await
            .unwrap()
            .is_empty()
    );
}
