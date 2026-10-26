use std::str::FromStr as _;

use super::SerializeError;
use crate::focus_records::table::{ColumnValues, FocusTable};

pub(super) fn encode(table: &FocusTable) -> Result<Vec<u8>, SerializeError> {
    let header: Vec<String> = table
        .columns
        .iter()
        .map(|column| quoted(column.name))
        .collect();
    let mut output = header.join(",").into_bytes();
    output.push(b'\n');
    for row in 0..table.rows {
        let fields = table
            .columns
            .iter()
            .map(|column| field(&column.values, row))
            .collect::<Result<Vec<_>, _>>()?;
        output.extend_from_slice(fields.join(",").as_bytes());
        output.push(b'\n');
    }
    Ok(output)
}

fn field(values: &ColumnValues, row: usize) -> Result<String, SerializeError> {
    match values {
        ColumnValues::Text(texts) => Ok(texts
            .get(row)
            .and_then(Option::as_deref)
            .map_or_else(String::new, quoted)),
        ColumnValues::Decimal(decimals) => decimals.get(row).copied().flatten().map_or_else(
            || Ok(String::new()),
            // LiteLLM casts Decimal to Float64 and polars prints the shortest float with ryu
            |value| {
                f64::from_str(&value.to_string())
                    .map(|float| ryu::Buffer::new().format(float).to_owned())
                    .map_err(|source| SerializeError::Float { value, source })
            },
        ),
        ColumnValues::Timestamp(times) => Ok(times
            .get(row)
            .copied()
            .flatten()
            .map_or_else(String::new, |time| {
                time.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
            })),
    }
}

// polars quotes only an empty string or one holding a comma, a quote or a line break, so null stays bare
fn quoted(text: &str) -> String {
    if text.is_empty() || text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}
