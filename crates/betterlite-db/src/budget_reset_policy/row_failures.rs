#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFailure {
    pub row: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowFailures {
    pub rows: Vec<RowFailure>,
}

impl RowFailures {
    pub fn record(&mut self, row: &str, error: &str) {
        self.rows.push(RowFailure {
            row: row.to_owned(),
            error: error.to_owned(),
        });
    }

    // The phase reports one error for the rows that failed, the way LiteLLM raises "Failed to reset N keys: ..." without failing the committed rows.
    #[must_use]
    pub fn summary(&self, subject: &str) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let details = self
            .rows
            .iter()
            .map(|failure| format!("{row}: {error}", row = failure.row, error = failure.error))
            .collect::<Vec<String>>()
            .join("; ");
        Some(format!(
            "Failed to reset {count} {subject}: {details}",
            count = self.rows.len()
        ))
    }
}

mod tests;
