fn new_row() -> NewGuardrailRow {
    NewGuardrailRow {
        guardrail_id: "g-1".to_owned(),
        created_at: now(),
        fields: update_row(),
    }
}

fn update_row() -> GuardrailUpdate {
    GuardrailUpdate {
        guardrail_name: "pii-guard".to_owned(),
        litellm_params: "{}".to_owned(),
        guardrail_info: "{}".to_owned(),
        updated_at: now(),
    }
}

fn adapter() -> PostgresGuardrailRows<'static> {
    let publish: PublishChange<TableError> = redis_config_change_publisher(None);
    PostgresGuardrailRows::new(None, publish)
}

#[tokio::test]
async fn rows_postgres_adapter_reports_the_missing_connection() {
    let rows = adapter();
    assert!(matches!(
        rows.create(new_row()).await,
        Err(RowsError::Database(_))
    ));
    assert!(matches!(
        rows.delete("g-1").await,
        Err(RowsError::Database(_))
    ));
    assert!(matches!(
        rows.update("g-1", update_row()).await,
        Err(RowsError::Database(_))
    ));
    assert!(matches!(
        rows.find_active().await,
        Err(RowsError::Database(_))
    ));
    assert!(matches!(
        rows.find_by_id("g-1").await,
        Err(RowsError::Database(_))
    ));
    assert!(matches!(
        rows.find_by_name("pii-guard").await,
        Err(RowsError::Database(_))
    ));
}
