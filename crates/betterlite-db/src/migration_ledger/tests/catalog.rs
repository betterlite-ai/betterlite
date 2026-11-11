#[rstest]
#[case::standard("20250326171002_add_daily_user_table", 20_250_326_171_002_i64)]
#[case::without_underscore("20250326171002", 0_i64)]
#[case::fifteen_digits("202503261710021_extra", 0_i64)]
#[case::thirteen_digits("2025032617100_short", 0_i64)]
#[case::eight_digit_email_index("20240101_email_index", 0_i64)]
#[case::leading_text("v20250326171002_x", 0_i64)]
#[case::trailing_letter("20250326171002x_name", 0_i64)]
fn timestamp_fourteen_digits(#[case] name: &str, #[case] expected: i64) {
    assert_eq!(migration_timestamp(name), expected);
}

#[test]
fn maximum_timestamp_over_the_catalog() {
    assert_eq!(
        max_migration_timestamp(Vec::<&str>::new().into_iter()),
        0_i64
    );
    assert_eq!(
        max_migration_timestamp(
            [
                "20240101000000_first",
                "unexpected_name",
                "20250101000000_last"
            ]
            .into_iter()
        ),
        20_250_101_000_000_i64
    );
    assert_eq!(
        max_migration_timestamp(["unexpected_name"].into_iter()),
        0_i64
    );
}

#[tokio::test]
async fn discovered_migration_names() {
    let names = get_migration_names(Path::new(env!("CARGO_MANIFEST_DIR")))
        .await
        .unwrap();
    assert_eq!(names.len(), 195);
    assert!(names.iter().all(|name| name.contains('_')));
    let missing = get_migration_names(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("absent")
            .as_path(),
    )
    .await
    .unwrap();
    assert!(missing.is_empty());
}
