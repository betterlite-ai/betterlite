#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn sync_without_an_id_returns_none_and_logs(#[case] id: Option<&str>) {
    let (captured, _guard) = capture();
    let mut handler = handler();
    let synced = handler.sync_guardrail_from_db(guardrail(id, "no-id", BEDROCK_PRE));
    assert!(synced.unwrap().is_none());
    assert!(held_ids(&handler).is_empty());
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.contains("Cannot sync guardrail without guardrail_id"),
        "{text}"
    );
}

#[test]
fn sync_initializes_a_new_row_with_the_db_source() {
    let (captured, _guard) = capture();
    let mut handler = handler();
    let synced = handler
        .sync_guardrail_from_db(guardrail(Some("row-1"), "pii", BEDROCK_PRE))
        .unwrap()
        .unwrap();
    assert_eq!(synced.guardrail_id.as_deref(), Some("row-1"));
    assert_eq!(
        handler.get_source("row-1"),
        Some(GuardrailDefinitionLocation::Db)
    );
    assert_eq!(handler.all_callbacks().count(), 1);
    let text = logged(&captured);
    assert!(
        text.contains("Guardrail 'pii' (ID: row-1) params changed, re-initializing..."),
        "{text}"
    );
}

#[test]
fn sync_changed_params_reinitializes_the_guardrail() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(Some("row-1"), "pii", BEDROCK_PRE),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let synced = handler
        .sync_guardrail_from_db(guardrail(Some("row-1"), "pii", BEDROCK_POST))
        .unwrap()
        .unwrap();
    assert_eq!(
        synced.litellm_params.mode,
        LitellmParamsMode::One("post_call".to_owned())
    );
    let held = handler.get_guardrail_by_id("row-1").unwrap();
    assert_eq!(held.litellm_params.mode, synced.litellm_params.mode);
    assert_eq!(
        handler.get_source("row-1"),
        Some(GuardrailDefinitionLocation::Db)
    );
    assert_eq!(handler.all_callbacks().count(), 1);
}

#[test]
fn sync_changed_params_that_fail_keep_the_previous_guardrail() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(Some("row-1"), "pii", BEDROCK_PRE),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let failed = handler.sync_guardrail_from_db(guardrail(Some("row-1"), "pii", UNSUPPORTED));
    assert!(matches!(failed, Err(RegistryError::InitFailed { .. })));
    let held = handler.get_guardrail_by_id("row-1").unwrap();
    assert_eq!(held.litellm_params.guardrail, "bedrock");
}

#[test]
fn sync_unchanged_params_keep_the_callbacks_and_set_the_source_to_db() {
    let mut handler = handler();
    let mut original = guardrail(Some("row-1"), "pii", BEDROCK_PRE);
    original.guardrail_info = Some(OpaqueParams(BTreeMap::from([(
        "owner".to_owned(),
        GuardrailValue::Text("config".to_owned()),
    )])));
    handler
        .initialize_guardrail(original, GuardrailDefinitionLocation::Config)
        .unwrap();
    let mut from_db = guardrail(Some("row-1"), "pii", BEDROCK_PRE);
    from_db.guardrail_info = Some(OpaqueParams(BTreeMap::from([(
        "owner".to_owned(),
        GuardrailValue::Text("db".to_owned()),
    )])));
    let synced = handler.sync_guardrail_from_db(from_db).unwrap().unwrap();
    // a rebuild would carry the new info, so the old info proves nothing was rebuilt
    let owner = synced.guardrail_info.unwrap().0["owner"].clone();
    assert_eq!(owner, GuardrailValue::Text("config".to_owned()));
    assert_eq!(
        handler.get_source("row-1"),
        Some(GuardrailDefinitionLocation::Db)
    );
    assert_eq!(handler.all_callbacks().count(), 1);
    let removed = handler.reconcile_db_guardrails(&BTreeSet::new());
    assert_eq!(removed, ["row-1"]);
}

#[test]
fn sync_unchanged_params_log_nothing_at_info() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(Some("row-1"), "pii", BEDROCK_PRE),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let (captured, _guard) = capture();
    handler
        .sync_guardrail_from_db(guardrail(Some("row-1"), "pii", BEDROCK_PRE))
        .unwrap();
    assert!(!logged(&captured).contains("re-initializing"));
}

#[test]
fn sync_a_renamed_guardrail_counts_as_changed() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(Some("row-1"), "pii", BEDROCK_PRE),
            GuardrailDefinitionLocation::Db,
        )
        .unwrap();
    let synced = handler
        .sync_guardrail_from_db(guardrail(Some("row-1"), "renamed", BEDROCK_PRE))
        .unwrap()
        .unwrap();
    assert_eq!(synced.guardrail_name, "renamed");
    assert_eq!(
        handler.get_guardrail_by_id("row-1").unwrap().guardrail_name,
        "renamed"
    );
}
