use std::io::BufRead;
use std::iter;

use super::error::BatchGuardrailError;
use super::record_json::load_record;
use super::records::ParsedRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLine {
    pub line_number: usize,
    pub bytes: Vec<u8>,
}

// Bytes, not text: json.loads sniffs the encoding itself, so a BOM or a lone surrogate the validation accepted still reads
pub fn iter_lines<Source>(
    mut file_source: Source,
) -> impl Iterator<Item = Result<RawLine, BatchGuardrailError>>
where
    Source: BufRead,
{
    let reads = iter::from_fn(move || {
        let mut bytes = Vec::new();
        let read = file_source.read_until(b'\n', &mut bytes);
        read.map(|count| (count > 0).then_some(bytes)).transpose()
    });
    // bytes.strip() also drops the vertical tab that u8::is_ascii_whitespace keeps
    let blank = |bytes: &[u8]| {
        bytes
            .iter()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C))
    };
    (1_usize..)
        .zip(reads)
        .filter_map(move |(line_number, read)| match read {
            Ok(bytes) if blank(bytes.as_slice()) => None,
            Ok(bytes) => Some(Ok(RawLine { line_number, bytes })),
            Err(source) => Some(Err(BatchGuardrailError::Read { source })),
        })
}

pub fn iter_records<Source>(
    file_source: Source,
) -> impl Iterator<Item = Result<ParsedRecord, BatchGuardrailError>>
where
    Source: BufRead,
{
    iter_lines(file_source).map(|line| {
        let RawLine { line_number, bytes } = line?;
        let payload = load_record(&bytes).map_err(|source| BatchGuardrailError::Record {
            line_number,
            source,
        })?;
        Ok(ParsedRecord {
            line_number,
            payload,
        })
    })
}
