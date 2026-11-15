#[derive(Default)]
struct Seen {
    create: Option<NewGuardrailRow>,
    delete: Option<String>,
    update: Option<(String, GuardrailUpdate)>,
    by_id: Option<String>,
    by_name: Option<String>,
}

#[derive(Default)]
struct FakeRows {
    create: Mutex<Option<Result<GuardrailRow, RowsError>>>,
    delete: Mutex<Option<Result<(), RowsError>>>,
    update: Mutex<Option<Result<Option<GuardrailRow>, RowsError>>>,
    active: Mutex<Option<Result<Vec<GuardrailRow>, RowsError>>>,
    by_id: Mutex<Option<Result<Option<GuardrailRow>, RowsError>>>,
    by_name: Mutex<Option<Result<Option<GuardrailRow>, RowsError>>>,
    seen: Arc<Mutex<Seen>>,
}

impl GuardrailRows for FakeRows {
    fn create(
        &self,
        row: NewGuardrailRow,
    ) -> impl Future<Output = Result<GuardrailRow, RowsError>> + Send {
        self.seen.lock().unwrap().create = Some(row);
        let scripted = self.create.lock().unwrap().take();
        std::future::ready(scripted.expect("create is scripted"))
    }

    fn delete(&self, guardrail_id: &str) -> impl Future<Output = Result<(), RowsError>> + Send {
        self.seen.lock().unwrap().delete = Some(guardrail_id.to_owned());
        let scripted = self.delete.lock().unwrap().take();
        std::future::ready(scripted.expect("delete is scripted"))
    }

    fn update(
        &self,
        guardrail_id: &str,
        row: GuardrailUpdate,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        self.seen.lock().unwrap().update = Some((guardrail_id.to_owned(), row));
        let scripted = self.update.lock().unwrap().take();
        std::future::ready(scripted.expect("update is scripted"))
    }

    fn find_active(&self) -> impl Future<Output = Result<Vec<GuardrailRow>, RowsError>> + Send {
        let scripted = self.active.lock().unwrap().take();
        std::future::ready(scripted.expect("find_active is scripted"))
    }

    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        self.seen.lock().unwrap().by_id = Some(guardrail_id.to_owned());
        let scripted = self.by_id.lock().unwrap().take();
        std::future::ready(scripted.expect("find_by_id is scripted"))
    }

    fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send {
        self.seen.lock().unwrap().by_name = Some(guardrail_name.to_owned());
        let scripted = self.by_name.lock().unwrap().take();
        std::future::ready(scripted.expect("find_by_name is scripted"))
    }
}

#[tokio::test]
async fn registry_add_returns_the_input_with_the_new_id() {
    let rows = FakeRows::default();
    let seen = Arc::clone(&rows.seen);
    *rows.create.lock().unwrap() = Some(Ok(row("db-1", "pii-guard")));
    let registry = GuardrailRegistry::new(rows);
    let at = now();
    let added = registry
        .add_guardrail_to_db(
            &guardrail(Some("client-id"), "pii-guard", BEDROCK_PARAMS),
            at,
        )
        .await
        .unwrap();
    assert_eq!(added.guardrail_id.as_deref(), Some("db-1"));
    assert_eq!(added.guardrail_name, "pii-guard");
    let written = seen.lock().unwrap().create.clone().unwrap();
    assert_ne!(written.guardrail_id, "client-id");
    assert_eq!(written.created_at, at);
    assert_eq!(written.updated_at, at);
    assert_eq!(written.guardrail_info, "{}");
}

#[tokio::test]
async fn registry_add_wraps_a_failure() {
    let rows = FakeRows::default();
    *rows.create.lock().unwrap() = Some(Err(RowsError::MissingRow));
    let registry = GuardrailRegistry::new(rows);
    let error = registry
        .add_guardrail_to_db(&guardrail(None, "pii-guard", BEDROCK_PARAMS), now())
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error adding guardrail to DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn registry_delete_returns_the_message() {
    let rows = FakeRows::default();
    let seen = Arc::clone(&rows.seen);
    *rows.delete.lock().unwrap() = Some(Ok(()));
    let registry = GuardrailRegistry::new(rows);
    let message = registry.delete_guardrail_from_db("g-1").await.unwrap();
    assert_eq!(message.message, "Guardrail g-1 deleted successfully");
    assert_eq!(seen.lock().unwrap().delete.as_deref(), Some("g-1"));
}

#[tokio::test]
async fn registry_delete_wraps_a_failure() {
    let rows = FakeRows::default();
    *rows.delete.lock().unwrap() = Some(Err(RowsError::MissingRow));
    let registry = GuardrailRegistry::new(rows);
    let error = registry.delete_guardrail_from_db("g-1").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error deleting guardrail from DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn registry_update_returns_the_stored_row() {
    let rows = FakeRows::default();
    let seen = Arc::clone(&rows.seen);
    *rows.update.lock().unwrap() = Some(Ok(Some(row("g-1", "renamed-guard"))));
    let registry = GuardrailRegistry::new(rows);
    let at = now();
    let updated = registry
        .update_guardrail_in_db(
            "g-1",
            &guardrail(Some("g-1"), "renamed-guard", BEDROCK_PARAMS),
            at,
        )
        .await
        .unwrap();
    assert_eq!(updated.guardrail_id.as_deref(), Some("g-1"));
    assert_eq!(updated.guardrail_name, "renamed-guard");
    assert_eq!(updated.created_at, Some(naive("2026-01-02T03:04:05").utc()));
    let (id, written) = seen.lock().unwrap().update.clone().unwrap();
    assert_eq!(id, "g-1");
    assert_eq!(written.updated_at, at);
    let decoded = decode_litellm_params(written.litellm_params.as_bytes()).unwrap();
    assert_eq!(decoded.guardrail, "bedrock");
}

#[tokio::test]
async fn registry_update_reports_a_missing_row() {
    let rows = FakeRows::default();
    *rows.update.lock().unwrap() = Some(Ok(None));
    let registry = GuardrailRegistry::new(rows);
    let error = registry
        .update_guardrail_in_db(
            "g-1",
            &guardrail(Some("g-1"), "pii-guard", BEDROCK_PARAMS),
            now(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error updating guardrail from DB: Guardrail not found, passed guardrail_id=g-1"
    );
}

#[tokio::test]
async fn registry_update_wraps_a_failure() {
    let rows = FakeRows::default();
    *rows.update.lock().unwrap() = Some(Err(RowsError::MissingRow));
    let registry = GuardrailRegistry::new(rows);
    let error = registry
        .update_guardrail_in_db(
            "g-1",
            &guardrail(Some("g-1"), "pii-guard", BEDROCK_PARAMS),
            now(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error updating guardrail from DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn registry_get_all_returns_the_rows_in_order() {
    let rows = FakeRows::default();
    let mut newer = row("g-new", "newer-guard");
    newer.created_at = naive("2026-01-02T03:04:05");
    let mut older = row("g-old", "older-guard");
    older.created_at = naive("2026-01-01T03:04:05");
    *rows.active.lock().unwrap() = Some(Ok(vec![newer, older]));
    let registry = GuardrailRegistry::new(rows);
    let listed = registry.get_all_guardrails_from_db().await.unwrap();
    let ids: Vec<&str> = listed
        .iter()
        .filter_map(|guardrail| guardrail.guardrail_id.as_deref())
        .collect();
    assert_eq!(ids, ["g-new", "g-old"]);
}

#[tokio::test]
async fn registry_get_all_wraps_a_failure() {
    let rows = FakeRows::default();
    *rows.active.lock().unwrap() = Some(Err(RowsError::MissingRow));
    let registry = GuardrailRegistry::new(rows);
    let error = registry.get_all_guardrails_from_db().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting guardrails from DB: guardrail row is missing"
    );
}

#[test]
fn registry_find_active_query_filters_and_orders() {
    let sql = super::postgres_rows::sql::select_active();
    assert!(sql.contains("WHERE status = 'active'"));
    assert!(sql.contains("ORDER BY created_at DESC"));
}

#[tokio::test]
async fn registry_reads_a_guardrail_by_id() {
    let rows = FakeRows::default();
    let seen = Arc::clone(&rows.seen);
    *rows.by_id.lock().unwrap() = Some(Ok(Some(row("g-1", "pii-guard"))));
    let registry = GuardrailRegistry::new(rows);
    let found = registry
        .get_guardrail_by_id_from_db("g-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.litellm_params.guardrail, "bedrock");
    assert_eq!(seen.lock().unwrap().by_id.as_deref(), Some("g-1"));
}

#[tokio::test]
async fn registry_reads_a_guardrail_by_name_and_returns_none_without_a_row() {
    let rows = FakeRows::default();
    *rows.by_name.lock().unwrap() = Some(Ok(Some(row("g-1", "pii-guard"))));
    *rows.by_id.lock().unwrap() = Some(Ok(None));
    let registry = GuardrailRegistry::new(rows);
    let found = registry
        .get_guardrail_by_name_from_db("pii-guard")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.guardrail_id.as_deref(), Some("g-1"));
    let missing = registry
        .get_guardrail_by_id_from_db("missing")
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn registry_read_wraps_a_failure() {
    let rows = FakeRows::default();
    *rows.by_id.lock().unwrap() = Some(Err(RowsError::MissingRow));
    let registry = GuardrailRegistry::new(rows);
    let error = registry
        .get_guardrail_by_id_from_db("g-1")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error getting guardrail from DB: guardrail row is missing"
    );
}

#[test]
fn registry_finds_the_first_callback_by_name() {
    let mut handler = handler();
    handler
        .initialize_guardrail(
            guardrail(None, "pii-guard", PRESIDIO_PARAMS),
            GuardrailDefinitionLocation::Config,
        )
        .unwrap();
    let registry = GuardrailRegistry::new(FakeRows::default());
    let callback = registry
        .get_initialized_guardrail_callback(&handler, "pii-guard")
        .unwrap();
    assert!(matches!(
        &callback.spec,
        CallbackSpec::Presidio(args) if args.role == PresidioRole::Scan && !args.apply_to_output
    ));
    assert!(
        registry
            .get_initialized_guardrail_callback(&handler, "other")
            .is_none()
    );
}
