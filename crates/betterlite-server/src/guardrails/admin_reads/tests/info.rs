#[tokio::test]
async fn info_serves_the_database_row_before_a_config_entry() {
    let setup = Setup {
        database: true,
        rows: vec![row(
            "dup",
            "db-guard",
            BEDROCK_SECRET,
            Some(r#"{"description":"from the row"}"#),
            None,
        )],
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("dup"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, "/guardrails/dup", Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(text(field(&parsed, "guardrail_name")), "db-guard");
    assert_eq!(text(field(&parsed, "guardrail_definition_location")), "db");
    assert!(
        text(field(&parsed, "created_at")).starts_with("2026-01-02T03:04:05"),
        "the db row keeps its timestamps"
    );
    assert_eq!(
        text(field(field(&parsed, "litellm_params"), "api_key")),
        "sk****34"
    );
    assert_eq!(
        text(field(field(&parsed, "guardrail_info"), "description")),
        "from the row"
    );
}

#[tokio::test]
async fn info_falls_back_to_a_config_entry() {
    let harness = harness(Setup::default());
    harness
        .seed(
            guardrail(Some("cfg-1"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, "/guardrails/cfg-1", Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(text(field(&parsed, "guardrail_name")), "config-guard");
    assert_eq!(
        text(field(&parsed, "guardrail_definition_location")),
        "config"
    );
    assert!(field(&parsed, "created_at").is_null());
    assert!(field(&parsed, "updated_at").is_null());
    assert!(
        field(&parsed, "guardrail_info")
            .as_object()
            .expect("an info object")
            .is_empty()
    );
}

#[tokio::test]
async fn info_serves_the_alias_path() {
    let harness = harness(Setup::default());
    harness
        .seed(
            guardrail(Some("cfg-1"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, "/guardrails/cfg-1/info", Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(field(&parse(&body), "guardrail_name")), "config-guard");
}

#[tokio::test]
async fn info_answers_404_with_the_exact_text() {
    let setup = Setup {
        database: true,
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, "/guardrails/missing", Some(admin())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail with ID missing not found");
}

#[tokio::test]
async fn info_ignores_a_db_sourced_memory_entry() {
    let setup = Setup {
        database: true,
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("stale"), "stale-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Db,
        )
        .await;
    let (status, body) = get(&harness, "/guardrails/stale", Some(admin())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail with ID stale not found");
}

#[tokio::test]
async fn info_without_a_database_uses_only_the_config() {
    let setup = Setup {
        database: false,
        rows: vec![row("dup", "db-guard", BEDROCK_PRE, None, None)],
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("dup"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, "/guardrails/dup", Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text(field(&parse(&body), "guardrail_name")), "config-guard");
    assert_eq!(harness.guardrails.find_reads(), 0);
}

#[tokio::test]
async fn info_a_database_failure_answers_500() {
    let setup = Setup {
        database: true,
        ..Setup::default()
    };
    let harness = harness(setup);
    harness.guardrails.fail_find();
    let (status, body) = get(&harness, "/guardrails/g1", Some(admin())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(detail(&body).contains("Error getting guardrail from DB"));
}

#[tokio::test]
async fn info_requires_a_valid_key() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, "/guardrails/cfg-1", None).await;
    assert_unauthorized(status, &body);
}
