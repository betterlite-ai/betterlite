#[tokio::test]
async fn setup_schema_failure_closes_fixture_leaving_database_unreachable() {
    let fixture = provision().await;
    let database_url = fixture.options.url.expose_secret().to_owned();
    let outcome = Box::pin(run_body(async move {
        initialize_or_close(fixture, "not valid sql;").await;
    }))
    .await;
    assert!(outcome.is_err());
    let reconnect = tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await;
    assert!(reconnect.is_err());
}
