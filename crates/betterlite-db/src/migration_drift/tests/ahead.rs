#[tokio::test]
async fn newer_db_warns() {
    Box::pin(with_schema(|database| async move {
        let client = database.pool.get().await.unwrap();
        create_ledger(&client).await;
        insert_ledger(&client).await;
        let (logs, dispatch) = capture();
        let _guard = set_default(&dispatch);
        let known = b709_names();
        let check = database.check();
        let hostile = warn_if_db_ahead_of_head(&check, &known).await;
        assert_eq!(hostile, ["20261101000000_future"]);
        let captured = logs.joined();
        assert!(
            captured.contains("newer than any migration this build ships"),
            "{captured}"
        );
        assert!(captured.contains("20261101000000_future"), "{captured}");
        assert!(!captured.contains("20200101000000_old"), "{captured}");
        assert!(!captured.contains("20261001100000_parallel"), "{captured}");
        assert!(!captured.contains("unexpected_name"), "{captured}");

        client
            .batch_execute("DROP TABLE _prisma_migrations")
            .await
            .unwrap();
        assert!(warn_if_db_ahead_of_head(&check, &known).await.is_empty());
        ahead_unavailable_is_silent(&known).await;
    }))
    .await;
}

#[test]
fn ahead_warning_truncates_after_five() {
    let known = vec!["20260101000000_known".to_owned()];
    let applied: Vec<String> = (1_u8..=6)
        .map(|index| format!("2027{index:02}01000000_later"))
        .collect();
    let hostile = hostile_names(&applied, &known);
    assert_eq!(hostile.len(), 6);
    let message = shown(&hostile);
    assert!(message.ends_with(" ..."), "{message}");
    assert_eq!(message.matches(", ").count(), 4);

    let five: Vec<String> = (1_u8..=5)
        .map(|index| format!("2027{index:02}01000000_later"))
        .collect();
    let five_hostile = hostile_names(&five, &known);
    assert_eq!(five_hostile.len(), 5);
    assert_eq!(shown(&five_hostile), five_hostile.join(", "));

    let malformed = hostile_names(
        &[
            "20200101000000_old".to_owned(),
            "unexpected_name".to_owned(),
        ],
        &known,
    );
    assert!(malformed.is_empty());
    let empty_head = hostile_names(&["unexpected_name".to_owned()], &[]);
    assert!(empty_head.is_empty());
    let duplicates = hostile_names(
        &[
            "20261101000000_future".to_owned(),
            "20261101000000_future".to_owned(),
        ],
        &known,
    );
    assert_eq!(duplicates, ["20261101000000_future"]);
}
