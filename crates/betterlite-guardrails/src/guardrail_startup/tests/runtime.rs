#[tokio::test]
async fn runtime_sync_locked_syncs_the_row_as_db() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let synced = runtime
        .sync_locked(guardrail(Some("row-1"), "pii", BEDROCK_PRE))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(synced.guardrail_id.as_deref(), Some("row-1"));
    let held = runtime.handler().read().await;
    assert_eq!(held_ids(&held), ["row-1"]);
    assert_eq!(
        held.get_source("row-1"),
        Some(GuardrailDefinitionLocation::Db)
    );
}

#[tokio::test]
async fn runtime_sync_locked_returns_the_registry_error() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let failed = runtime
        .sync_locked(guardrail(Some("row-1"), "pii", UNSUPPORTED))
        .await;
    assert!(matches!(failed, Err(RegistryError::InitFailed { .. })));
    assert!(
        runtime
            .handler()
            .read()
            .await
            .list_in_memory_guardrails()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn runtime_sync_locked_waits_for_the_reconcile_mutex_before_the_write_lock() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let busy = runtime.reconcile_lock.lock().await;
    let pending = timeout(
        Duration::from_millis(50),
        runtime.sync_locked(guardrail(Some("row-1"), "pii", BEDROCK_PRE)),
    );
    let probe = async {
        yield_now().await;
        runtime.handler().try_write().is_ok()
    };
    let (outcome, write_lock_was_free) = tokio::join!(pending, probe);
    assert!(outcome.is_err(), "sync ran while the mutex was held");
    assert!(
        write_lock_was_free,
        "sync took the write lock before the mutex"
    );
    drop(busy);
    runtime
        .sync_locked(guardrail(Some("row-1"), "pii", BEDROCK_PRE))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn runtime_sync_locked_holds_the_mutex_while_it_waits_for_the_write_lock() {
    let (runtime, _reads) = runtime_with(handler(), Vec::new());
    let reader = runtime.handler().read().await;
    let pending = timeout(
        Duration::from_millis(50),
        runtime.sync_locked(guardrail(Some("row-1"), "pii", BEDROCK_PRE)),
    );
    let probe = async {
        yield_now().await;
        runtime.reconcile_lock.try_lock().is_err()
    };
    let (outcome, mutex_was_taken) = tokio::join!(pending, probe);
    assert!(outcome.is_err(), "sync wrote while a reader held the lock");
    assert!(
        mutex_was_taken,
        "sync waited for the write lock without the mutex"
    );
    drop(reader);
}

#[tokio::test]
async fn runtime_exposes_the_registry_it_was_built_with() {
    let polls = vec![Ok(vec![row("row-1", "pii", BEDROCK_PRE)])];
    let (runtime, reads) = runtime_with(handler(), polls);
    let rows = runtime
        .registry()
        .get_all_guardrails_from_db()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}
