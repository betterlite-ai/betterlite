#![cfg(test)]

use std::io::Cursor;

use super::{FocusFormat, SerializeError};
use crate::focus_records::schema::FOCUS_NORMALIZED_SCHEMA;
use crate::focus_records::table::{ColumnValues, FocusColumn, FocusTable};
use crate::focus_records::transformer::transform;
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
    let time = NaiveDate::from_ymd_opt(2024, 3, 5)
        .unwrap()
        .and_hms_micro_opt(0, 0, 0, 7);
    let costs = vec![
        Some(Decimal::new(1_500_000, 6)),
        Some(Decimal::new(12, 6)),
        Some(Decimal::new(3, 0)),
        None,
    ];
    let names = vec![
        Some("plain".to_owned()),
        Some("a,b \"q\"".to_owned()),
        Some(String::new()),
        None,
    ];
    let columns = vec![
        FocusColumn {
            name: "Cost",
            values: ColumnValues::Decimal(costs),
        },
        FocusColumn {
            name: "Name",
            values: ColumnValues::Text(names),
        },
        FocusColumn {
            name: "When",
            values: ColumnValues::Timestamp(vec![time, None, None, None]),
        },
    ];
    FocusTable { rows: 4, columns }
}

fn single(values: ColumnValues) -> FocusTable {
    FocusTable {
        rows: 1,
        columns: vec![FocusColumn {
            name: "Cost",
            values,
        }],
    }
}

fn metadata(bytes: &[u8]) -> FileMetaData {
    read_metadata(&mut Cursor::new(bytes)).unwrap()
}

fn column_page(bytes: &[u8], index: usize) -> (Vec<u32>, Vec<u8>) {
    let file = metadata(bytes);
    let chunk = &file.row_groups[0].columns()[index];
    let mut pages =
        get_page_iterator(chunk, Cursor::new(bytes), None, Vec::new(), usize::MAX).unwrap();
    let compressed = pages.next().unwrap().unwrap();
    assert!(pages.next().is_none());
    let Page::Data(page) = decompress(compressed, &mut Vec::new()).unwrap() else {
        panic!("column {index} has no data page");
    };
    let (_, levels, values) = split_buffer(&page).unwrap();
    let decoder = HybridRleDecoder::try_new(levels, 1, page.num_values()).unwrap();
    (
        decoder.collect::<Result<Vec<u32>, _>>().unwrap(),
        values.to_vec(),
    )
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
    let expected = "Cost,Name,When\n1.5,plain,2024-03-05T00:00:00.000007\n0.000012,\"a,b \"\"q\"\"\",\n3.0,\"\",\n,,\n";
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[rstest]
#[case::one_millionth(Decimal::new(1, 6), "1e-6")]
#[case::ten_millionths(Decimal::new(10, 6), "0.00001")]
#[case::negative(Decimal::new(-2_250_000, 6), "-2.25")]
#[case::large(Decimal::new(999_999_999_999_500_000, 6), "999999999999.5")]
fn csv_prints_the_shortest_float(#[case] value: Decimal, #[case] expected: &str) {
    let bytes = FocusFormat::Csv
        .encode(&single(ColumnValues::Decimal(vec![Some(value)])))
        .unwrap();
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
    assert_eq!(
        (
            file.num_rows,
            file.row_groups.len(),
            file.schema_descr.columns().len()
        ),
        (0, 0, 37)
    );
}

#[rstest]
#[case::billed_cost(0, PhysicalType::Int64, PrimitiveLogicalType::Decimal(18, 6))]
#[case::billing_account(1, PhysicalType::ByteArray, PrimitiveLogicalType::String)]
#[case::period_start(
    4,
    PhysicalType::Int64,
    PrimitiveLogicalType::Timestamp { unit: TimeUnit::Microseconds, is_adjusted_to_utc: false }
)]
#[case::tags(36, PhysicalType::ByteArray, PrimitiveLogicalType::String)]
fn parquet_keeps_the_schema_of_an_empty_table(
    #[case] index: usize,
    #[case] physical: PhysicalType,
    #[case] logical: PrimitiveLogicalType,
) {
    let bytes = FocusFormat::Parquet
        .encode(&transform(&[]).unwrap())
        .unwrap();
    let file = metadata(&bytes);
    let primitive = &file.schema_descr.columns()[index].descriptor.primitive_type;
    assert_eq!(primitive.field_info.name, FOCUS_NORMALIZED_SCHEMA[index].0);
    assert_eq!(primitive.field_info.repetition, Repetition::Optional);
    assert_eq!(
        (primitive.physical_type, primitive.logical_type),
        (physical, Some(logical))
    );
}

#[test]
fn parquet_writes_one_snappy_row_group() {
    let file = metadata(&FocusFormat::Parquet.encode(&sample()).unwrap());
    assert_eq!(
        (file.num_rows, file.created_by.as_deref()),
        (4, Some("betterlite"))
    );
    let group = &file.row_groups[0];
    let compressions: Vec<Compression> = group
        .columns()
        .iter()
        .map(ColumnChunkMetaData::compression)
        .collect();
    assert_eq!(
        (group.num_rows(), compressions),
        (4, vec![Compression::Snappy; 3])
    );
}

#[rstest]
#[case::decimals(0, vec![1, 1, 1, 0], [1_500_000_i64, 12, 3_000_000].iter().flat_map(|value| value.to_le_bytes()).collect())]
#[case::text(1, vec![1, 1, 1, 0], b"\x05\0\0\0plain\x07\0\0\0a,b \"q\"\0\0\0\0".to_vec())]
#[case::timestamps(2, vec![1, 0, 0, 0], 1_709_596_800_000_007_i64.to_le_bytes().to_vec())]
fn parquet_stores_plain_values_behind_definition_levels(
    #[case] index: usize,
    #[case] levels: Vec<u32>,
    #[case] values: Vec<u8>,
) {
    let bytes = FocusFormat::Parquet.encode(&sample()).unwrap();
    assert_eq!(column_page(&bytes, index), (levels, values));
}

#[test]
fn parquet_rejects_a_decimal_past_64_bits() {
    let table = single(ColumnValues::Decimal(vec![Some(Decimal::new(
        10_000_000_000_000_000,
        0,
    ))]));
    let error = FocusFormat::Parquet.encode(&table).unwrap_err();
    assert!(matches!(error, SerializeError::Decimal { .. }), "{error:?}");
}

#[rstest]
#[case::csv(FocusFormat::Csv, "csv")]
#[case::parquet(FocusFormat::Parquet, "parquet")]
fn formats_name_their_extension(#[case] format: FocusFormat, #[case] expected: &str) {
    assert_eq!(format.extension(), expected);
}
