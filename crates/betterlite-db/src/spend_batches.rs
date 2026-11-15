use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

mod tests;

const STATEMENT_FRAMING_BYTES: usize = 2;
const ROW_SEPARATOR_BYTES: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatementLimits {
    pub max_bytes: usize,
    pub max_rows: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrimmedQueue<'rows, Row> {
    pub rows: &'rows [Row],
    pub queued_bytes: usize,
}

// json.dumps bytes: default separators, every character outside printable ASCII escaped
#[must_use]
pub fn spend_log_row_bytes<Row>(row: &Row) -> usize
where
    Row: Serialize,
{
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, PythonJson);
    if row.serialize(&mut serializer).is_err() {
        return 0;
    }
    bytes.len()
}

// Slices keep the order, cover every row once, and hold a row over the byte budget on its own
#[must_use]
pub fn spend_log_write_batches<Row>(rows: &[Row], limits: StatementLimits) -> Vec<&[Row]>
where
    Row: Serialize,
{
    let sizes: Vec<usize> = rows.iter().map(spend_log_row_bytes).collect();
    let mut batches = Vec::new();
    let mut start = 0;
    while let Some(size) = sizes.get(start).copied() {
        let mut used = STATEMENT_FRAMING_BYTES.saturating_add(size);
        let mut end = start.saturating_add(1);
        while let Some(next) = sizes.get(end).copied() {
            let Some(grown) = used
                .checked_add(ROW_SEPARATOR_BYTES)
                .and_then(|fits| fits.checked_add(next))
            else {
                break;
            };
            if end.saturating_sub(start) >= limits.max_rows || grown > limits.max_bytes {
                break;
            }
            used = grown;
            end = end.saturating_add(1);
        }
        batches.push(rows.get(start..end).unwrap_or_default());
        start = end;
    }
    batches
}

// Drops the oldest rows until the running total fits, and keeps the newest however large it is
#[must_use]
pub fn spend_log_queue_within_budget<Row>(
    rows: &[Row],
    queued_bytes: usize,
    max_bytes: usize,
) -> TrimmedQueue<'_, Row>
where
    Row: Serialize,
{
    if queued_bytes <= max_bytes || rows.len() <= 1 {
        return TrimmedQueue { rows, queued_bytes };
    }
    let Some((newest, droppable)) = rows.split_last() else {
        return TrimmedQueue { rows, queued_bytes };
    };
    let mut remaining = queued_bytes;
    for (index, row) in droppable.iter().enumerate() {
        remaining = remaining.saturating_sub(spend_log_row_bytes(row));
        if remaining <= max_bytes {
            return TrimmedQueue {
                rows: rows.get(index.saturating_add(1)..).unwrap_or_default(),
                queued_bytes: remaining,
            };
        }
    }
    TrimmedQueue {
        rows: rows.get(rows.len().saturating_sub(1)..).unwrap_or_default(),
        queued_bytes: spend_log_row_bytes(newest),
    }
}

fn separator<W>(writer: &mut W, first: bool) -> io::Result<()>
where
    W: ?Sized + io::Write,
{
    if first {
        return Ok(());
    }
    writer.write_all(b", ")
}

// LiteLLM measures json.dumps(row, default=str), whose ensure_ascii output the write budget charges
struct PythonJson;

impl Formatter for PythonJson {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        for unit in fragment.encode_utf16() {
            match u8::try_from(unit) {
                Ok(byte) if byte.is_ascii_graphic() || byte == b' ' => writer.write_all(&[byte])?,
                Ok(_) | Err(_) => write!(writer, "\\u{unit:04x}")?,
            }
        }
        Ok(())
    }

    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        separator(writer, first)
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        separator(writer, first)
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}
