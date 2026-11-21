#[tokio::test]
async fn list_reads_the_config_guardrails_with_masked_params() {
    let setup = Setup {
        database: false,
        rows: Vec::new(),
        config: vec![config_entry(
            Some("cfg-1"),
            "my-guard",
            Some(BEDROCK_SECRET),
            Some(r#"{"description":"guard"}"#),
        )],
        teams: Ok(Vec::new()),
        env: BTreeMap::new(),
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let entry = listed_entries(&parsed).first().expect("a config entry");
    assert_eq!(text(field(entry, "guardrail_id")), "cfg-1");
    assert_eq!(text(field(entry, "guardrail_name")), "my-guard");
    assert_eq!(
        text(field(entry, "guardrail_definition_location")),
        "config"
    );
    assert!(field(entry, "created_at").is_null());
    assert!(field(entry, "updated_at").is_null());
    assert_eq!(
        text(field(field(entry, "guardrail_info"), "description")),
        "guard"
    );
    let params = field(entry, "litellm_params");
    assert_eq!(text(field(params, "api_key")), "sk****34");
    assert_eq!(text(field(params, "guardrail")), "bedrock");
    assert_eq!(text(field(params, "mode")), "pre_call");
    // a config entry carries no provider mixin defaults
    assert!(params.get("disable_exception_on_block").is_none());
}

#[tokio::test]
async fn list_answers_empty_when_the_config_has_no_guardrails() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, LIST_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"guardrails":[]}"#);
}

#[tokio::test]
async fn list_gives_default_params_when_an_item_has_none() {
    let setup = Setup {
        database: false,
        rows: Vec::new(),
        config: vec![config_entry(Some("cfg-1"), "my-guard", None, None)],
        teams: Ok(Vec::new()),
        env: BTreeMap::new(),
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let entry = &listed_entries(&parsed)[0];
    let params = field(entry, "litellm_params");
    let fields = params.as_object().expect("the default params");
    assert!(!fields.contains_key("guardrail"));
    assert!(field(params, "default_on").is_null());
    assert_eq!(field(params, "fail_on_error"), &JsonValue::Bool(true));
    assert!(field(entry, "guardrail_info").is_null());
}

#[tokio::test]
async fn list_shows_a_missing_id_and_info_as_null() {
    let setup = Setup {
        database: false,
        rows: Vec::new(),
        config: vec![config_entry(None, "my-guard", Some(BEDROCK_PRE), None)],
        teams: Ok(Vec::new()),
        env: BTreeMap::new(),
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let entry = &listed_entries(&parsed)[0];
    assert!(field(entry, "guardrail_id").is_null());
    assert!(field(entry, "guardrail_info").is_null());
}

#[tokio::test]
async fn list_never_reads_the_database() {
    let setup = Setup {
        database: true,
        rows: vec![row("db-1", "db-guard", BEDROCK_PRE, None, None)],
        config: vec![config_entry(Some("cfg-1"), "my-guard", None, None)],
        teams: Ok(Vec::new()),
        env: BTreeMap::new(),
    };
    let harness = harness(setup);
    harness.guardrails.fail_active();
    let (status, body) = get(&harness, LIST_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(entry_ids(&parsed), ["cfg-1"]);
    assert_eq!(harness.guardrails.active_reads(), 0);
}

#[tokio::test]
async fn list_requires_a_valid_key() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, LIST_PATH, None).await;
    assert_unauthorized(status, &body);
}

#[test]
fn list_empty_masked_params_stay_null() {
    assert!(
        validated(&JsonObject::new())
            .expect("an empty map")
            .is_none()
    );
    let mut present = JsonObject::new();
    present.insert(
        "guardrail".to_owned(),
        JsonValue::String("bedrock".to_owned()),
    );
    assert!(validated(&present).expect("a masked map").is_some());
}
