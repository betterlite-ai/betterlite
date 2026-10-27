use std::iter;

use parquet_format_safe::Encoding;
use parquet2::compression::CompressionOptions;
use parquet2::encoding::hybrid_rle::encode_bool;
use parquet2::metadata::{Descriptor, SchemaDescriptor};
use parquet2::page::{DataPage, DataPageHeader, DataPageHeaderV1, Page};
use parquet2::schema::Repetition;
use parquet2::schema::types::{
    FieldInfo, ParquetType, PhysicalType, PrimitiveConvertedType, PrimitiveLogicalType,
    PrimitiveType, TimeUnit,
};
use parquet2::write::{
    Compressor, DynIter, DynStreamingIterator, FileWriter, Version, WriteOptions,
};
use rust_decimal::Decimal;

use super::SerializeError;
use crate::focus_records::schema::{ColumnType, DECIMAL_PRECISION, DECIMAL_SCALE};
use crate::focus_records::table::{ColumnValues, FocusColumn, FocusTable};

pub(super) fn encode(table: &FocusTable) -> Result<Vec<u8>, SerializeError> {
    let fields = table
        .columns
        .iter()
        .map(|column| ParquetType::PrimitiveType(primitive(column)))
        .collect();
    let schema = SchemaDescriptor::new("schema".to_owned(), fields);
    let descriptors: Vec<Descriptor> = schema
        .columns()
        .iter()
        .map(|column| column.descriptor.clone())
        .collect();
    let options = WriteOptions {
        write_statistics: false,
        version: Version::V1,
    };
    let mut writer = FileWriter::new(Vec::new(), schema, options, Some("betterlite".to_owned()));
    // an empty table writes no row group, so only the footer carries its schema
    if table.rows > 0 {
        let pages = table
            .columns
            .iter()
            .zip(descriptors)
            .map(|(column, descriptor)| page(&column.values, descriptor, table.rows))
            .collect::<Result<Vec<_>, _>>()?;
        let chunks = pages.into_iter().map(|page| {
            let compressor = Compressor::new(
                DynIter::new(iter::once(Ok(page))),
                CompressionOptions::Snappy,
                Vec::new(),
            );
            Ok(DynStreamingIterator::new(compressor))
        });
        writer
            .write(DynIter::new(chunks))
            .map_err(SerializeError::Parquet)?;
    }
    writer.end(None).map_err(SerializeError::Parquet)?;
    Ok(writer.into_inner())
}

fn primitive(column: &FocusColumn) -> PrimitiveType {
    let precision = usize::from(DECIMAL_PRECISION);
    let scale = usize::from(DECIMAL_SCALE);
    let (physical_type, logical_type, converted_type) = match column.values.kind() {
        ColumnType::Decimal => (
            PhysicalType::Int64,
            PrimitiveLogicalType::Decimal(precision, scale),
            Some(PrimitiveConvertedType::Decimal(precision, scale)),
        ),
        ColumnType::Text => (
            PhysicalType::ByteArray,
            PrimitiveLogicalType::String,
            Some(PrimitiveConvertedType::Utf8),
        ),
        // polars writes Datetime("us") without a time zone, so the value is not adjusted to UTC
        ColumnType::Timestamp => (
            PhysicalType::Int64,
            PrimitiveLogicalType::Timestamp {
                unit: TimeUnit::Microseconds,
                is_adjusted_to_utc: false,
            },
            None,
        ),
    };
    PrimitiveType {
        field_info: FieldInfo {
            name: column.name.to_owned(),
            repetition: Repetition::Optional,
            id: None,
        },
        logical_type: Some(logical_type),
        converted_type,
        physical_type,
    }
}

fn page(
    values: &ColumnValues,
    descriptor: Descriptor,
    rows: usize,
) -> Result<Page, SerializeError> {
    let cells = plain_cells(values)?;
    let mut levels = Vec::new();
    encode_bool(&mut levels, cells.iter().map(Option::is_some)).map_err(SerializeError::Levels)?;
    let levels_length = u32::try_from(levels.len()).map_err(SerializeError::Length)?;
    let mut buffer = levels_length.to_le_bytes().to_vec();
    buffer.append(&mut levels);
    buffer.extend(cells.into_iter().flatten().flatten());
    let header = DataPageHeaderV1 {
        num_values: i32::try_from(rows).map_err(SerializeError::Length)?,
        encoding: Encoding::PLAIN,
        definition_level_encoding: Encoding::RLE,
        repetition_level_encoding: Encoding::RLE,
        statistics: None,
    };
    Ok(Page::Data(DataPage::new(
        DataPageHeader::V1(header),
        buffer,
        descriptor,
        Some(rows),
    )))
}

fn plain_cells(values: &ColumnValues) -> Result<Vec<Option<Vec<u8>>>, SerializeError> {
    match values {
        ColumnValues::Text(texts) => texts
            .iter()
            .map(|text| text.as_deref().map(byte_array).transpose())
            .collect(),
        ColumnValues::Decimal(decimals) => decimals
            .iter()
            .map(|decimal| decimal.map(unscaled).transpose())
            .collect(),
        ColumnValues::Timestamp(times) => Ok(times
            .iter()
            .map(|time| time.map(|value| value.and_utc().timestamp_micros().to_le_bytes().to_vec()))
            .collect()),
    }
}

fn byte_array(text: &str) -> Result<Vec<u8>, SerializeError> {
    let length = u32::try_from(text.len()).map_err(SerializeError::Length)?;
    Ok([length.to_le_bytes().as_slice(), text.as_bytes()].concat())
}

fn unscaled(value: Decimal) -> Result<Vec<u8>, SerializeError> {
    let mut scaled = value;
    // rescale stops short of six places only for values whose mantissa is already past i64
    scaled.rescale(u32::from(DECIMAL_SCALE));
    let mantissa = i64::try_from(scaled.mantissa())
        .map_err(|source| SerializeError::Decimal { value, source })?;
    Ok(mantissa.to_le_bytes().to_vec())
}
