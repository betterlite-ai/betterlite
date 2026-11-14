use super::support::run;
use crate::daily_activity_repository::rows::{DatabaseRow, RowError};
use rust_decimal::Decimal;

#[tokio::test]
async fn postgres_decode_every_column_type() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let rows = client
            .query(
                "SELECT 'a'::text AS t, 'b'::varchar AS v, 'c'::bpchar AS b, 'd'::name AS n,
                        8::int8 AS i8, 4::int4 AS i4, 2::int2 AS i2,
                        1.5::float8 AS f8, 0.25::float4 AS f4,
                        NULL::text AS nt, NULL::int8 AS ni, NULL::float8 AS nf",
                &[],
            )
            .await
            .unwrap();
        let row = DatabaseRow::decode(&rows[0]).unwrap();
        assert_eq!(row.text("t").unwrap(), "a");
        assert_eq!(row.text("v").unwrap(), "b");
        assert_eq!(row.text("b").unwrap(), "c");
        assert_eq!(row.text("n").unwrap(), "d");
        assert_eq!(row.integer("i8").unwrap(), 8);
        assert_eq!(row.integer("i4").unwrap(), 4);
        assert_eq!(row.integer("i2").unwrap(), 2);
        assert_eq!(row.decimal("f8").unwrap(), Decimal::new(15, 1));
        assert_eq!(row.decimal("f4").unwrap(), Decimal::new(25, 2));
        assert_eq!(row.optional_text("nt").unwrap(), None);
        assert_eq!(row.optional_integer("ni").unwrap(), None);
        assert_eq!(row.optional_decimal("nf").unwrap(), None);
        assert_eq!(row.optional_text("t").unwrap().as_deref(), Some("a"));
        assert_eq!(row.optional_integer("i8").unwrap(), Some(8));
        assert_eq!(row.integer_if_present("i8").unwrap(), Some(8));
        assert_eq!(row.integer_if_present("ni").unwrap(), None);
        assert_eq!(row.integer_if_present("absent").unwrap(), None);
        assert!(matches!(
            row.text("absent"),
            Err(RowError::MissingColumn { .. })
        ));
        assert!(matches!(row.text("nt"), Err(RowError::ColumnType { .. })));
        assert!(matches!(
            row.integer("ni"),
            Err(RowError::ColumnType { .. })
        ));
        assert!(matches!(
            row.decimal("nf"),
            Err(RowError::ColumnType { .. })
        ));
        assert!(matches!(row.text("i8"), Err(RowError::ColumnType { .. })));
        assert!(matches!(row.integer("t"), Err(RowError::ColumnType { .. })));
        assert!(matches!(row.decimal("t"), Err(RowError::ColumnType { .. })));
        assert!(matches!(
            row.integer_if_present("t"),
            Err(RowError::ColumnType { .. })
        ));
    })
    .await;
}

#[tokio::test]
async fn postgres_decode_rejects_unsupported_and_non_finite() {
    run(|pool| async move {
        let client = pool.get().await.unwrap();
        let unsupported = client.query("SELECT true AS flag", &[]).await.unwrap();
        assert!(matches!(
            DatabaseRow::decode(&unsupported[0]),
            Err(RowError::UnsupportedColumn { .. })
        ));
        let non_finite = client
            .query("SELECT 'NaN'::float8 AS number", &[])
            .await
            .unwrap();
        assert!(matches!(
            DatabaseRow::decode(&non_finite[0]),
            Err(RowError::NonFiniteNumber { .. })
        ));
        let narrow = client
            .query("SELECT 'Infinity'::float4 AS number", &[])
            .await
            .unwrap();
        assert!(matches!(
            DatabaseRow::decode(&narrow[0]),
            Err(RowError::NonFiniteNumber { .. })
        ));
    })
    .await;
}
