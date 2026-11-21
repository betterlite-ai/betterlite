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
    let mut line_number = 0_usize;
    iter::from_fn(move || {
        loop {
            let mut bytes = Vec::new();
            match file_source.read_until(b'\n', &mut bytes) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(source) => return Some(Err(BatchGuardrailError::Read { source })),
            }
            line_number = line_number.saturating_add(1);
            // bytes.strip() also drops the vertical tab that u8::is_ascii_whitespace keeps
            let blank = bytes
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0B | 0x0C));
            if !blank {
                return Some(Ok(RawLine { line_number, bytes }));
            }
        }
    })
}

pub fn iter_records<Source>(
    file_source: Source,
) -> impl Iterator<Item = Result<ParsedRecord, BatchGuardrailError>>
where
    Source: BufRead,
{
    iter_lines(file_source).map(|line| {
        let raw = line?;
        let payload = load_record(&raw.bytes).map_err(|source| BatchGuardrailError::Record {
            line_number: raw.line_number,
            source,
        })?;
        Ok(ParsedRecord {
            line_number: raw.line_number,
            payload,
        })
    })
}
