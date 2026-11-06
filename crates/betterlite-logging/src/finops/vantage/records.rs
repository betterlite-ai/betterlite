#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Start,
    Plain,
    Quoted,
    AfterQuote,
}

// A newline inside a quoted cell belongs to the record, so only newlines outside quotes split.
#[must_use]
pub fn split_records(content: &[u8]) -> Vec<&[u8]> {
    let mut records = Vec::new();
    let mut record_start = 0;
    let mut field = Field::Start;
    for (index, byte) in content.iter().enumerate() {
        field = match (field, byte) {
            (Field::Quoted, b'"') => Field::AfterQuote,
            (Field::Quoted, _) | (Field::AfterQuote | Field::Start, b'"') => Field::Quoted,
            (_, b',') => Field::Start,
            (_, b'\n') => {
                records.push(content.get(record_start..index).unwrap_or_default());
                record_start = index.saturating_add(1);
                Field::Start
            }
            _ => Field::Plain,
        };
    }
    records.push(content.get(record_start..).unwrap_or_default());
    records
}

#[must_use]
pub fn is_blank(record: &[u8]) -> bool {
    record
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c'))
}
