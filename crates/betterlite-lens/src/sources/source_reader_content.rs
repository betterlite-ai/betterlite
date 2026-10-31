use crate::models::execution::{Execution, TracePart};
use crate::sources::part_row::PartRow;

pub(crate) const CONTENT_PAGE_SIZE: usize = 40;

pub(crate) fn trace_parts(execution: &Execution, rows: &[PartRow]) -> Vec<TracePart> {
    let mut parts = Vec::with_capacity(rows.len());
    for row in rows {
        parts.push(TracePart {
            execution_id: execution.id.clone(),
            span_id: row.span_id.clone(),
            parent_span_id: row.parent_span_id.clone(),
            name: row.name.clone(),
            kind: row.kind.clone(),
            content: row.content.clone(),
            truncated: row.truncated != 0,
        });
    }
    parts
}

mod tests;
