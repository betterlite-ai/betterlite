#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EndUserWalk {
    pub(crate) cursor: Option<String>,
    pub(crate) invalidated: usize,
    pub(crate) truncated: bool,
}

impl EndUserWalk {
    // One page of rows: a short page ends the walk, and a full one resumes strictly after its last user_id.
    #[must_use]
    pub(crate) fn after_page(
        &self,
        last_user_id: Option<&str>,
        rows: usize,
        batch_size: usize,
    ) -> Self {
        let invalidated = self.invalidated.saturating_add(rows);
        let cursor = if rows < batch_size {
            None
        } else {
            last_user_id.map(str::to_owned)
        };
        Self {
            cursor,
            invalidated,
            truncated: false,
        }
    }

    // _invalidate_enduser_page's failure branch: the cursor is dropped, the reached count stands, and the tail keeps its cached spend until it expires.
    #[must_use]
    pub(crate) const fn truncated_at(&self) -> Self {
        Self {
            cursor: None,
            invalidated: self.invalidated,
            truncated: true,
        }
    }
}

mod tests;
