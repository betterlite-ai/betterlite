#[test]
fn rows_read_the_json_text_stored_by_litellm() {
    let mut stored = row("g-1", "pii-guard");
    stored.litellm_params = Some(serde_json::to_string(BEDROCK_PARAMS).unwrap());
    let guardrail = row_to_guardrail(&stored).unwrap();
    assert_eq!(guardrail.guardrail_id.as_deref(), Some("g-1"));
    assert_eq!(guardrail.guardrail_name, "pii-guard");
    assert_eq!(guardrail.litellm_params.guardrail, "bedrock");
    assert_eq!(guardrail.created_at, Some(stored.created_at.utc()));
}

#[test]
fn rows_read_an_object_value_too() {
    let stored = row("g-2", "object-guard");
    let guardrail = row_to_guardrail(&stored).unwrap();
    assert_eq!(
        guardrail.litellm_params.mode,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn rows_carry_team_id_into_the_guardrail() {
    let mut stored = row("g-3", "team-guard");
    stored.team_id = Some("team-9".to_owned());
    let guardrail = row_to_guardrail(&stored).unwrap();
    assert_eq!(guardrail.team_id.as_deref(), Some("team-9"));
    assert_eq!(guardrail.updated_at, Some(stored.updated_at.utc()));
}

fn info_team(row: &GuardrailRow) -> Option<GuardrailValue> {
    let guardrail = row_to_guardrail(row).unwrap();
    let info = guardrail.guardrail_info?;
    info.0.get("team").cloned()
}

#[test]
fn rows_read_guardrail_info_in_both_shapes() {
    let mut typed = row("g-4", "info-guard");
    typed.guardrail_info = Some(serde_json::to_string(r#"{"team":"x"}"#).unwrap());
    let mut plain = typed.clone();
    plain.guardrail_info = Some(r#"{"team":"x"}"#.to_owned());
    assert_eq!(
        info_team(&typed),
        Some(GuardrailValue::Text("x".to_owned()))
    );
    assert_eq!(
        info_team(&plain),
        Some(GuardrailValue::Text("x".to_owned()))
    );
}

#[test]
fn rows_without_params_are_an_error() {
    let mut stored = row("g-5", "empty-guard");
    stored.litellm_params = None;
    let error = row_to_guardrail(&stored).unwrap_err();
    assert_eq!(
        error.to_string(),
        "stored guardrail row has no litellm_params"
    );
}

#[test]
fn rows_decode_errors_do_not_show_parameter_values() {
    let mut stored = row("g-6", "secret-guard");
    stored.litellm_params = Some(
        r#"{"guardrail":"bedrock","mode":"pre_call","api_key":"sk-secret-value","timeout":"soon"}"#
            .to_owned(),
    );
    let error = row_to_guardrail(&stored).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("timeout"));
    assert!(!message.contains("sk-secret-value"));
}

#[test]
fn rows_new_row_defaults_guardrail_info_and_uses_a_db_id() {
    let created = guardrail_to_new_row(
        &guardrail(Some("client-id"), "pii-guard", BEDROCK_PARAMS),
        now(),
    )
    .unwrap();
    assert_ne!(created.guardrail_id, "client-id");
    assert_eq!(created.guardrail_id.len(), 36);
    assert_eq!(created.guardrail_info, "{}");
    assert_eq!(created.created_at, now());
    assert_eq!(created.updated_at, now());
    let decoded = decode_litellm_params(created.litellm_params.as_bytes()).unwrap();
    assert_eq!(decoded.guardrail, "bedrock");
}

#[test]
fn rows_new_row_writes_the_given_info() {
    let mut with_info = guardrail(None, "pii-guard", BEDROCK_PARAMS);
    with_info.guardrail_info = Some(OpaqueParams(BTreeMap::from([(
        "team".to_owned(),
        GuardrailValue::Text("x".to_owned()),
    )])));
    let created = guardrail_to_new_row(&with_info, now()).unwrap();
    assert!(created.guardrail_info.contains("\"team\""));
}

#[test]
fn rows_update_sets_updated_at() {
    let updated =
        guardrail_to_update(&guardrail(Some("g-1"), "pii-guard", BEDROCK_PARAMS), now()).unwrap();
    assert_eq!(updated.guardrail_name, "pii-guard");
    assert_eq!(updated.updated_at, now());
    assert_eq!(updated.guardrail_info, "{}");
    let decoded = decode_litellm_params(updated.litellm_params.as_bytes()).unwrap();
    assert_eq!(decoded.mode, LitellmParamsMode::One("pre_call".to_owned()));
}

#[test]
fn rows_sql_stores_json_text_and_reads_it_back() {
    let insert = super::postgres_rows::sql::insert();
    assert!(insert.contains("to_jsonb($3::text)"));
    let active = super::postgres_rows::sql::select_active();
    assert!(active.contains("'litellm_params', litellm_params::text"));
}
