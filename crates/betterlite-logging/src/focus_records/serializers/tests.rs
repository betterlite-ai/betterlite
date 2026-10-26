#![cfg(test)]

use std::io::Cursor;

use super::{FocusFormat, SerializeError};
use crate::focus_records::schema::FOCUS_NORMALIZED_SCHEMA;
use crate::focus_records::table::{ColumnValues, FocusColumn, FocusTable};
use crate::focus_records::transformer::{UsageRow, transform};
use chrono::NaiveDate;
use parquet2::compression::Compression;
use parquet2::encoding::hybrid_rle::HybridRleDecoder;
use parquet2::metadata::{ColumnChunkMetaData, FileMetaData};
use parquet2::page::{Page, split_buffer};
use parquet2::read::{decompress, get_page_iterator, read_metadata};
use parquet2::schema::Repetition;
use parquet2::schema::types::{PhysicalType, PrimitiveLogicalType, TimeUnit};
use rstest::rstest;
use rust_decimal::Decimal;

fn sample() -> FocusTable {
    let midnight = NaiveDate::from_ymd_opt(2024, 3, 5)
        .unwrap()
        .and_hms_micro_opt(0, 0, 0, 7)
        .unwrap();
    FocusTable {
        rows: 4,
        columns: vec![
            FocusColumn {
                name: "Cost",
                values: ColumnValues::Decimal(vec![
                    Some(Decimal::new(1_500_000, 6)),
                    Some(Decimal::new(12, 6)),
                    Some(Decimal::new(3, 0)),
                    None,
                ]),
            },
            FocusColumn {
                name: "Name",
                values: ColumnValues::Text(vec![
                    Some("plain".to_owned()),
                    Some("a,b \"q\"".to_owned()),
                    Some(String::new()),
                    None,
                ]),
            },
            FocusColumn {
                name: "When",
                values: ColumnValues::Timestamp(vec![Some(midnight), None, None, None]),
            },
        ],
    }
}

fn metadata(bytes: &[u8]) -> FileMetaData {
    read_metadata(&mut Cursor::new(bytes)).unwrap()
}

fn column_page(bytes: &[u8], index: usize) -> (Vec<u32>, Vec<u8>) {
    let file = metadata(bytes);
    let chunk = file
        .row_groups
        .first()
        .unwrap()
        .columns()
        .get(index)
        .unwrap();
    let mut pages =
        get_page_iterator(chunk, Cursor::new(bytes), None, Vec::new(), usize::MAX).unwrap();
    let compressed = pages.next().unwrap().unwrap();
    assert!(pages.next().is_none());
    let Page::Data(page) = decompress(compressed, &mut Vec::new()).unwrap() else {
        panic!("column {index} has no data page");
    };
    let (_, levels, values) = split_buffer(&page).unwrap();
    let decoded = HybridRleDecoder::try_new(levels, 1, page.num_values())
        .unwrap()
        .collect::<Result<Vec<u32>, _>>()
        .unwrap();
    (decoded, values.to_vec())
}

#[test]
fn csv_writes_header_only_for_an_empty_table() {
    let bytes = FocusFormat::Csv.encode(&transform(&[]).unwrap()).unwrap();
    let names: Vec<&str> = FOCUS_NORMALIZED_SCHEMA
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!("{}\n", names.join(","))
    );
}

#[test]
fn csv_casts_decimals_to_floats_and_quotes_like_polars() {
    let bytes = FocusFormat::Csv.encode(&sample()).unwrap();
    let expected = concat!(
        "Cost,Name,When\n",
        "1.5,plain,2024-03-05T00:00:00.000007\n",
        "0.000012,\"a,b \"\"q\"\"\",\n",
        "3.0,\"\",\n",
        ",,\n",
    );
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[rstest]
#[case::one_millionth(Decimal::new(1, 6), "1e-6")]
#[case::ten_millionths(Decimal::new(10, 6), "0.00001")]
#[case::negative(Decimal::new(-2_250_000, 6), "-2.25")]
#[case::large(Decimal::new(999_999_999_999_500_000, 6), "999999999999.5")]
fn csv_prints_the_shortest_float(#[case] value: Decimal, #[case] expected: &str) {
    let table = FocusTable {
        rows: 1,
        columns: vec![FocusColumn {
            name: "Cost",
            values: ColumnValues::Decimal(vec![Some(value)]),
        }],
    };
    let bytes = FocusFormat::Csv.encode(&table).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        format!("Cost\n{expected}\n")
    );
}

#[test]
fn parquet_writes_no_row_group_for_an_empty_table() {
    let bytes = FocusFormat::Parquet
        .encode(&transform(&[]).unwrap())
        .unwrap();
    assert!(bytes.starts_with(b"PAR1") && bytes.ends_with(b"PAR1"));
    let file = metadata(&bytes);
    assert_eq!(file.num_rows, 0);
    assert!(file.row_groups.is_empty());
    assert_eq!(file.schema_descr.columns().len(), 37);
}

#[rstest]
#[case::billed_cost(
    0,
    "BilledCost",
    PhysicalType::Int64,
    PrimitiveLogicalType::Decimal(18, 6)
)]
#[case::billing_account(
    1,
    "BillingAccountId",
    PhysicalType::ByteArray,
    PrimitiveLogicalType::String
)]
#[case::period_start(
    4,
    "BillingPeriodStart",
    PhysicalType::Int64,
    PrimitiveLogicalType::Timestamp { unit: TimeUnit::Microseconds, is_adjusted_to_utc: false }
)]
#[case::tags(36, "Tags", PhysicalType::ByteArray, PrimitiveLogicalType::String)]
fn parquet_keeps_the_schema_of_an_empty_table(
    #[case] index: usize,
    #[case] name: &str,
    #[case] physical: PhysicalType,
    #[case] logical: PrimitiveLogicalType,
) {
    let bytes = FocusFormat::Parquet
        .encode(&transform(&[]).unwrap())
        .unwrap();
    let file = metadata(&bytes);
    let primitive = &file.schema_descr.columns()[index].descriptor.primitive_type;
    assert_eq!(primitive.field_info.name, name);
    assert_eq!(primitive.field_info.repetition, Repetition::Optional);
    assert_eq!(primitive.physical_type, physical);
    assert_eq!(primitive.logical_type, Some(logical));
}

#[test]
fn parquet_writes_one_snappy_row_group() {
    let bytes = FocusFormat::Parquet.encode(&sample()).unwrap();
    let file = metadata(&bytes);
    assert_eq!(file.num_rows, 4);
    assert_eq!(file.created_by.as_deref(), Some("betterlite"));
    let group = file.row_groups.first().unwrap();
    assert_eq!(group.num_rows(), 4);
    let compressions: Vec<Compression> = group
        .columns()
        .iter()
        .map(ColumnChunkMetaData::compression)
        .collect();
    assert_eq!(compressions, vec![Compression::Snappy; 3]);
}

#[test]
fn parquet_stores_decimals_as_scaled_integers() {
    let bytes = FocusFormat::Parquet.encode(&sample()).unwrap();
    let (levels, values) = column_page(&bytes, 0);
    assert_eq!(levels, vec![1, 1, 1, 0]);
    let expected: Vec<u8> = [1_500_000_i64, 12, 3_000_000]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    assert_eq!(values, expected);
}

#[test]
fn parquet_stores_text_with_length_prefixes() {
    let bytes = FocusFormat::Parquet.encode(&sample()).unwrap();
    let (levels, values) = column_page(&bytes, 1);
    assert_eq!(levels, vec![1, 1, 1, 0]);
    let expected = [
        b"\x05\0\0\0plain".as_slice(),
        b"\x07\0\0\0a,b \"q\"".as_slice(),
        b"\0\0\0\0".as_slice(),
    ]
    .concat();
    assert_eq!(values, expected);
}

#[test]
fn parquet_stores_timestamps_as_microseconds() {
    let bytes = FocusFormat::Parquet.encode(&sample()).unwrap();
    let (levels, values) = column_page(&bytes, 2);
    assert_eq!(levels, vec![1, 0, 0, 0]);
    assert_eq!(values, 1_709_596_800_000_007_i64.to_le_bytes().to_vec());
}

#[test]
fn parquet_rejects_a_decimal_past_64_bits() {
    let table = FocusTable {
        rows: 1,
        columns: vec![FocusColumn {
            name: "Cost",
            values: ColumnValues::Decimal(vec![Some(Decimal::new(10_000_000_000_000_000, 0))]),
        }],
    };
    let error = FocusFormat::Parquet.encode(&table).unwrap_err();
    assert!(matches!(error, SerializeError::Decimal { .. }), "{error:?}");
}

#[rstest]
#[case::csv(FocusFormat::Csv, "csv")]
#[case::parquet(FocusFormat::Parquet, "parquet")]
fn formats_name_their_extension(#[case] format: FocusFormat, #[case] expected: &str) {
    assert_eq!(format.extension(), expected);
}

#[test]
fn transformed_rows_round_trip_through_csv() {
    let row = UsageRow {
        date: "2024-03-05".to_owned(),
        api_key: "key".to_owned(),
        spend: 0.25,
        api_requests: 2,
        user_id: None,
        model: Some("m".to_owned()),
        model_group: None,
        custom_llm_provider: None,
        team_id: Some("t".to_owned()),
        team_alias: None,
        organization_id: None,
        organization_alias: None,
        user_email: None,
        api_key_alias: None,
    };
    let bytes = FocusFormat::Csv
        .encode(&transform(&[row]).unwrap())
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let line = text.lines().nth(1).unwrap();
    let expected = concat!(
        "0.25,key,,API Key,USD,2024-03-06T00:00:00Z,2024-03-05T00:00:00Z,Usage,,m,Usage-Based,",
        "2024-03-06T00:00:00Z,2024-03-05T00:00:00Z,2.0,Requests,0.25,,0.25,,,0.25,,,USD,,2.0,,0.25,,",
        "Requests,,,,,m,m,m,AI and Machine Learning,Generative AI,,t,,,",
        "\"{\"\"team_id\"\": \"\"t\"\", \"\"model\"\": \"\"m\"\"}\"",
    );
    assert_eq!(line, expected);
}
