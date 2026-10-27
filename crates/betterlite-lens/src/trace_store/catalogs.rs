use super::{CatalogRow, TracePart, TraceStoreError, overview_content};

pub(super) struct Catalogs<Parts> {
    pub(super) parts: Parts,
    pub(super) pending: Option<CatalogRow>,
    pub(super) root_count: usize,
}

impl<Parts> Iterator for Catalogs<Parts>
where
    Parts: Iterator<Item = Result<TracePart, TraceStoreError>>,
{
    type Item = Result<Vec<CatalogRow>, TraceStoreError>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut page = Vec::new();
        let mut size = 2_usize;
        loop {
            let row = match self.pending.take() {
                Some(row) => row,
                None => match self.parts.next() {
                    None => break,
                    Some(Ok(part)) => catalog_row(part, self.root_count),
                    Some(Err(error)) => return Some(Err(error)),
                },
            };
            let width = match serde_json::to_vec(&row) {
                Ok(json) => json.len(),
                Err(error) => return Some(Err(TraceStoreError::Json(error))),
            };
            if width > 23_998 {
                return Some(Err(TraceStoreError::Oversized));
            }
            let added = width.saturating_add(usize::from(!page.is_empty()));
            if size.saturating_add(added) > 24_000 {
                self.pending = Some(row);
                break;
            }
            size = size.saturating_add(added);
            page.push(row);
        }
        if page.is_empty() {
            None
        } else {
            Some(Ok(page))
        }
    }
}

fn catalog_row(part: TracePart, root_count: usize) -> CatalogRow {
    let preview = overview_content(&part, root_count);
    (
        part.span_id,
        part.parent_span_id,
        part.name,
        part.kind,
        preview,
    )
}
