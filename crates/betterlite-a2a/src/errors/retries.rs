#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetryCounts {
    pub retried: Option<u32>,
    pub max: Option<u32>,
}

impl RetryCounts {
    #[must_use]
    pub fn render(&self) -> String {
        match (self.retried, self.max) {
            (None, None) => String::new(),
            (Some(retried), None) => format!(" (retried {retried} times)"),
            (None, Some(max)) => format!(" (max retries {max})"),
            (Some(retried), Some(max)) => {
                format!(" (retried {retried} times, max retries {max})")
            }
        }
    }
}
