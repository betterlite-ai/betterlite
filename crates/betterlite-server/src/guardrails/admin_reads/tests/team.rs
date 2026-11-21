#[tokio::test]
async fn team_admin_sees_every_row_and_skips_the_team_lookup() {
    let setup = Setup {
        database: true,
        rows: vec![
            row("row1", "one", BEDROCK_PRE, None, Some("team-a")),
            row("row2", "two", BEDROCK_PRE, None, Some("team-b")),
        ],
        teams: Err("the lookup must not run"),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(role_admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row1", "row2"]);
    assert_eq!(harness.teams.calls(), 0);
}

#[tokio::test]
async fn team_a_viewer_role_sees_every_row() {
    let setup = Setup {
        database: true,
        rows: vec![row("row1", "one", BEDROCK_PRE, None, Some("team-a"))],
        teams: Err("the lookup must not run"),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(viewer())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row1"]);
    assert_eq!(harness.teams.calls(), 0);
}

#[tokio::test]
async fn team_the_master_key_sees_every_row_and_skips_the_team_lookup() {
    let setup = Setup {
        database: true,
        rows: vec![
            row("row1", "one", BEDROCK_PRE, None, Some("team-a")),
            row("row2", "two", BEDROCK_PRE, None, Some("team-b")),
        ],
        teams: Err("the lookup must not run"),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row1", "row2"]);
    assert_eq!(harness.teams.calls(), 0);
}

#[tokio::test]
async fn team_a_non_admin_sees_its_own_team_memory_entry() {
    let setup = Setup {
        database: true,
        teams: Ok(vec!["team-a".to_owned()]),
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed_team_scoped(
            guardrail(Some("mem-a"), "own-guard", BEDROCK_PRE, None),
            "team-a",
        )
        .await;
    harness
        .seed_team_scoped(
            guardrail(Some("mem-b"), "other-guard", BEDROCK_PRE, None),
            "team-b",
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["mem-a"]);
    assert_eq!(harness.teams.calls(), 1);
}

#[tokio::test]
async fn team_a_non_admin_sees_own_and_teamless_rows() {
    let setup = Setup {
        database: true,
        rows: vec![
            row("row1", "one", BEDROCK_PRE, None, Some("team-a")),
            row("row2", "two", BEDROCK_PRE, None, Some("team-b")),
            row("row3", "three", BEDROCK_PRE, None, None),
        ],
        teams: Ok(vec!["team-a".to_owned()]),
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("cfg-1"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row1", "row3", "cfg-1"]);
    assert_eq!(harness.teams.calls(), 1);
}

#[tokio::test]
async fn team_a_hidden_row_keeps_its_id_hidden_from_config() {
    let setup = Setup {
        database: true,
        rows: vec![row("shared", "db-guard", BEDROCK_PRE, None, Some("team-b"))],
        teams: Ok(vec!["team-a".to_owned()]),
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("shared"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(entry_ids(&parse(&body)).is_empty());
}

#[tokio::test]
async fn team_an_empty_team_id_matches_no_caller() {
    let setup = Setup {
        database: true,
        rows: vec![
            row("row1", "one", BEDROCK_PRE, None, Some("")),
            row("row2", "two", BEDROCK_PRE, None, Some("team-a")),
        ],
        teams: Ok(vec!["team-a".to_owned(), String::new()]),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row2"]);
}

#[tokio::test]
async fn team_a_caller_without_a_user_id_sees_only_teamless_rows() {
    let setup = Setup {
        database: true,
        rows: vec![
            row("row1", "one", BEDROCK_PRE, None, Some("team-a")),
            row("row2", "two", BEDROCK_PRE, None, None),
        ],
        teams: Ok(vec!["team-a".to_owned()]),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(anonymous())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["row2"]);
    assert_eq!(harness.teams.calls(), 0);
}

#[tokio::test]
async fn team_without_a_database_adds_no_rows_and_skips_the_lookup() {
    let setup = Setup {
        database: false,
        rows: vec![row("row1", "one", BEDROCK_PRE, None, None)],
        teams: Err("the lookup must not run"),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(entry_ids(&parse(&body)).is_empty());
    assert_eq!(harness.guardrails.active_reads(), 0);
    assert_eq!(harness.teams.calls(), 0);
}

#[tokio::test]
async fn team_skips_a_db_sourced_memory_entry() {
    let harness = harness(Setup::default());
    harness
        .seed(
            guardrail(Some("stale"), "stale-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Db,
        )
        .await;
    harness
        .seed(
            guardrail(Some("cfg-1"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(entry_ids(&parse(&body)), ["cfg-1"]);
}

#[tokio::test]
async fn team_never_repeats_an_id() {
    let setup = Setup {
        database: true,
        rows: vec![row("dup", "db-guard", BEDROCK_PRE, None, None)],
        config: vec![config_entry(Some("dup"), "config-guard", None, None)],
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("dup"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(entry_ids(&parsed), ["dup"]);
    assert_eq!(
        text(field(
            &listed_entries(&parsed)[0],
            "guardrail_definition_location"
        )),
        "db"
    );
}

#[tokio::test]
async fn team_labels_db_and_config_entries() {
    let setup = Setup {
        database: true,
        rows: vec![row("db-1", "db-guard", BEDROCK_SECRET, None, None)],
        ..Setup::default()
    };
    let harness = harness(setup);
    harness
        .seed(
            guardrail(Some("cfg-1"), "config-guard", BEDROCK_PRE, None),
            GuardrailDefinitionLocation::Config,
        )
        .await;
    let (status, body) = get(&harness, LIST_V2_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let listed = listed_entries(&parsed);
    assert_eq!(listed.len(), 2);
    let database_entry = &listed[0];
    assert_eq!(
        text(field(database_entry, "guardrail_definition_location")),
        "db"
    );
    assert!(
        text(field(database_entry, "created_at")).starts_with("2026-01-02T03:04:05"),
        "the db entry keeps its timestamps"
    );
    assert!(field(database_entry, "guardrail_info").is_null());
    assert_eq!(
        text(field(field(database_entry, "litellm_params"), "api_key")),
        "sk****34"
    );
    let config_entry = &listed[1];
    assert_eq!(
        text(field(config_entry, "guardrail_definition_location")),
        "config"
    );
    assert!(field(config_entry, "created_at").is_null());
    assert!(
        field(config_entry, "guardrail_info")
            .as_object()
            .expect("an info object")
            .is_empty()
    );
}

#[tokio::test]
async fn team_a_database_failure_answers_500_and_logs() {
    let (captured, _guard) = capture();
    let setup = Setup {
        database: true,
        ..Setup::default()
    };
    let harness = harness(setup);
    harness.guardrails.fail_active();
    let (status, body) = get(&harness, LIST_V2_PATH, Some(admin())).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(detail(&body).contains("Error getting guardrails from DB"));
    assert!(
        logged(&captured).contains("Error getting guardrails from db: "),
        "the failure is logged: {}",
        logged(&captured)
    );
}

#[tokio::test]
async fn team_a_team_lookup_failure_answers_500() {
    let setup = Setup {
        database: true,
        teams: Err("no teams reached"),
        ..Setup::default()
    };
    let harness = harness(setup);
    let (status, body) = get(&harness, LIST_V2_PATH, Some(user_caller("user-1"))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(detail(&body), "no teams reached");
}

#[tokio::test]
async fn team_requires_a_valid_key() {
    let harness = harness(Setup::default());
    let (status, body) = get(&harness, LIST_V2_PATH, None).await;
    assert_unauthorized(status, &body);
}
