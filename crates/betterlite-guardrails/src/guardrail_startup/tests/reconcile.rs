fn initialized(
    handler: &mut InMemoryGuardrailHandler,
    id: &str,
    source: GuardrailDefinitionLocation,
) {
    let entry = guardrail(Some(id), id, BEDROCK_PRE);
    handler.initialize_guardrail(entry, source).unwrap();
}

#[test]
fn reconcile_removes_db_entries_missing_from_the_set_in_the_order_found() {
    let mut handler = handler();
    for id in ["first", "second", "third", "fourth"] {
        initialized(&mut handler, id, GuardrailDefinitionLocation::Db);
    }
    let kept = BTreeSet::from(["second".to_owned()]);
    assert_eq!(
        handler.reconcile_db_guardrails(&kept),
        ["first", "third", "fourth"]
    );
    assert_eq!(held_ids(&handler), ["second"]);
    assert_eq!(handler.all_callbacks().count(), 1);
}

#[test]
fn reconcile_never_touches_config_guardrails() {
    let mut handler = handler();
    initialized(
        &mut handler,
        "from-file",
        GuardrailDefinitionLocation::Config,
    );
    initialized(&mut handler, "from-db", GuardrailDefinitionLocation::Db);
    let removed = handler.reconcile_db_guardrails(&BTreeSet::new());
    assert_eq!(removed, ["from-db"]);
    assert_eq!(held_ids(&handler), ["from-file"]);
    assert_eq!(
        handler.get_source("from-file"),
        Some(GuardrailDefinitionLocation::Config)
    );
}

#[test]
fn reconcile_keeps_every_db_entry_the_database_still_has() {
    let mut handler = handler();
    initialized(&mut handler, "kept", GuardrailDefinitionLocation::Db);
    let present = BTreeSet::from(["kept".to_owned(), "elsewhere".to_owned()]);
    assert!(handler.reconcile_db_guardrails(&present).is_empty());
    assert_eq!(held_ids(&handler), ["kept"]);
}

#[test]
fn reconcile_logs_each_removal_at_info() {
    let (captured, _guard) = capture();
    let mut handler = handler();
    initialized(&mut handler, "gone", GuardrailDefinitionLocation::Db);
    handler.reconcile_db_guardrails(&BTreeSet::new());
    let text = logged(&captured);
    assert!(text.contains("INFO"), "{text}");
    assert!(
        text.contains("Reconcile: removing stale DB-backed guardrail 'gone' from memory (deleted in DB by another pod)"),
        "{text}"
    );
}
