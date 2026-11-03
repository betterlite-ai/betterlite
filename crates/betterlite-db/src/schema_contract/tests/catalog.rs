#[tokio::test]
async fn postgres_independent_catalogs() {
    harness(|[reference, candidate]| async move {
        let expected = SchemaContract::read(&**reference, "public", LIMIT)
            .await
            .unwrap();
        let actual = SchemaContract::read(&**candidate, "public", LIMIT)
            .await
            .unwrap();
        let report = expected.compare(&actual);
        assert!(report.differences.is_empty(), "{:#?}", report.differences);
        assert!(expected.facts.len() > 10_000);
    })
    .await;
}
