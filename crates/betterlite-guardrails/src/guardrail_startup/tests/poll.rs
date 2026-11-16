fn seeded(ids: &[(&str, GuardrailDefinitionLocation)]) -> InMemoryGuardrailHandler {
    let mut store = handler();
    for &(id, source) in ids {
        let entry = guardrail(Some(id), id, BEDROCK_PRE);
        store.initialize_guardrail(entry, source).unwrap();
    }
    store
}

#[tokio::test]
async fn poll_syncs_each_active_row_as_a_db_guardrail() {
    let polls = vec![Ok(vec![
        row("row-1", "pii", BEDROCK_PRE),
        row("row-2", "secrets", BEDROCK_POST),
    ])];
    let (runtime, reads) = runtime_with(handler(), polls);
    poll_database(&runtime).await;
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    let held = runtime.handler().read().await;
    assert_eq!(held_ids(&held), ["row-1", "row-2"]);
    assert_eq!(
        held.get_source("row-2"),
        Some(GuardrailDefinitionLocation::Db)
    );
}

#[tokio::test]
async fn poll_skips_a_bad_row_logs_it_and_syncs_the_rest() {
    let (captured, _guard) = capture();
    let polls = vec![Ok(vec![
        row("row-1", "pii", BEDROCK_PRE),
        row("bad-row", "broken", UNSUPPORTED),
        row("row-3", "secrets", BEDROCK_PRE),
    ])];
    let (runtime, _reads) = runtime_with(handler(), polls);
    poll_database(&runtime).await;
    assert_eq!(runtime_ids(&runtime).await, ["row-1", "row-3"]);
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.contains("skipping guardrail 'broken' (ID: bad-row): "),
        "{text}"
    );
}

#[tokio::test]
async fn poll_reconciles_stale_db_entries_and_leaves_config_ones() {
    let seed = seeded(&[
        ("stale", GuardrailDefinitionLocation::Db),
        ("from-file", GuardrailDefinitionLocation::Config),
        ("kept", GuardrailDefinitionLocation::Db),
    ]);
    let polls = vec![Ok(vec![row("kept", "kept", BEDROCK_PRE)])];
    let (runtime, _reads) = runtime_with(seed, polls);
    poll_database(&runtime).await;
    assert_eq!(runtime_ids(&runtime).await, ["from-file", "kept"]);
}

#[tokio::test]
async fn poll_with_an_empty_table_purges_every_db_entry() {
    let seed = seeded(&[("stale", GuardrailDefinitionLocation::Db)]);
    let (runtime, _reads) = runtime_with(seed, vec![Ok(Vec::new())]);
    poll_database(&runtime).await;
    assert!(runtime_ids(&runtime).await.is_empty());
}

#[tokio::test]
async fn poll_failed_read_leaves_memory_alone_and_logs() {
    let (captured, _guard) = capture();
    let seed = seeded(&[
        ("db-entry", GuardrailDefinitionLocation::Db),
        ("from-file", GuardrailDefinitionLocation::Config),
    ]);
    let (runtime, reads) = runtime_with(seed, vec![Err(RowsError::MissingRow)]);
    poll_database(&runtime).await;
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(runtime_ids(&runtime).await, ["db-entry", "from-file"]);
    let text = logged(&captured);
    assert!(text.contains("ERROR"), "{text}");
    assert!(text.contains("Error getting guardrails from DB"), "{text}");
}

#[tokio::test]
async fn poll_keeps_the_previous_entry_of_a_row_that_fails_to_sync() {
    let seed = seeded(&[("row-1", GuardrailDefinitionLocation::Db)]);
    let polls = vec![Ok(vec![row("row-1", "row-1", UNSUPPORTED)])];
    let (runtime, _reads) = runtime_with(seed, polls);
    poll_database(&runtime).await;
    let held = runtime.handler().read().await;
    assert_eq!(held_ids(&held), ["row-1"]);
    assert_eq!(
        held.get_guardrail_by_id("row-1")
            .unwrap()
            .litellm_params
            .guardrail,
        "bedrock"
    );
}

#[tokio::test(start_paused = true)]
async fn poll_waits_for_the_reconcile_mutex_before_it_reads() {
    let polls = vec![Ok(vec![row("row-1", "pii", BEDROCK_PRE)])];
    let (runtime, reads) = runtime_with(handler(), polls);
    let busy = runtime.reconcile_lock.lock().await;
    let waited = timeout(Duration::from_millis(50), poll_database(&runtime)).await;
    assert!(waited.is_err());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    drop(busy);
    poll_database(&runtime).await;
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}
