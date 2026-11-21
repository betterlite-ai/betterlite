fn nested_depth(model: &FormModel, depth: usize) -> usize {
    let mut deepest = 0;
    for field in &model.fields {
        if let FormShape::Nested(inner) = &field.shape {
            deepest = deepest.max(depth);
            deepest = deepest.max(nested_depth(inner, depth.saturating_add(1)));
        }
    }
    deepest
}

fn deepest_nested() -> usize {
    let mut deepest = 0;
    let tables = core_tables()
        .into_iter()
        .chain(tables_a())
        .chain(tables_b());
    for (_key, model) in tables {
        deepest = deepest.max(nested_depth(&model, 0));
    }
    deepest
}

fn depth_setup(depth: usize) -> Setup {
    let mut setup = Setup::default();
    setup
        .env
        .insert("DEFAULT_MAX_RECURSE_DEPTH".to_owned(), depth.to_string());
    setup
}

#[tokio::test]
async fn forms_returns_the_merged_provider_map() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let providers = parsed.as_object().expect("the provider map");
    assert_eq!(providers.len(), 52);
    for key in [
        "bedrock",
        "presidio",
        "lakera_v2",
        "tool_permission",
        "hide-secrets",
        "custom_code",
        "azure/prompt_shield",
        "azure/text_moderations",
        "zscaler_ai_guard",
        "litellm_content_filter",
    ] {
        assert!(providers.contains_key(key), "{key} is present");
    }
    assert!(
        field(&parsed, "bedrock")
            .as_object()
            .expect("the bedrock fields")
            .contains_key("guardrailIdentifier")
    );
}

#[tokio::test]
async fn forms_sets_the_friendly_name_on_the_registry_entries() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert!(
        field(&parsed, "presidio")
            .as_object()
            .expect("the presidio fields")
            .get("ui_friendly_name")
            .is_none()
    );
    assert_eq!(
        text(field(field(&parsed, "hide-secrets"), "ui_friendly_name")),
        "Hide Secrets"
    );
    assert_eq!(
        text(field(field(&parsed, "tool_permission"), "ui_friendly_name")),
        "LiteLLM Tool Permission Guardrail"
    );
    assert_eq!(
        text(field(field(&parsed, "aporia"), "ui_friendly_name")),
        "Aporia AI"
    );
    assert!(field(field(&parsed, "custom_code"), "ui_friendly_name").is_null());
}

#[tokio::test]
async fn forms_answers_400_at_the_depth_limit() {
    let limit = deepest_nested();
    let harness = harness(depth_setup(limit));
    let (status, body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let expected = format!(
        "Max depth of {limit} exceeded while processing model fields. Please check the model structure for excessive nesting."
    );
    assert_eq!(detail(&body), expected);
}

#[tokio::test]
async fn forms_answers_200_just_above_the_depth_limit() {
    let limit = deepest_nested().saturating_add(1);
    let harness = harness(depth_setup(limit));
    let (status, _body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forms_uses_the_default_depth() {
    assert!(
        deepest_nested().saturating_add(1) < 100,
        "the deepest model fits the default depth"
    );
    let harness = harness(Setup::default());
    let (status, _body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn forms_answers_500_for_an_unreadable_depth() {
    let mut setup = Setup::default();
    setup.env.insert(
        "DEFAULT_MAX_RECURSE_DEPTH".to_owned(),
        "not-a-number".to_owned(),
    );
    let harness = harness(setup);
    let (status, body) = get(&harness, FORMS_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(detail(&body).contains("DEFAULT_MAX_RECURSE_DEPTH"));
}

#[tokio::test]
async fn forms_requires_a_valid_key() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, FORMS_PATH, None).await;
    assert_unauthorized(status, &body);
}
