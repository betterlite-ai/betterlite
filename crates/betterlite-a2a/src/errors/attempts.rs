use crate::card_resolver::fetch_error::ProbeFailure;

#[derive(Debug)]
pub struct ProbeAttempts {
    pub failures: Vec<ProbeFailure>,
}

impl ProbeAttempts {
    #[must_use]
    pub fn render(&self) -> String {
        self.failures
            .iter()
            .map(|failure| format!("{} ({})", failure.path, failure.reason))
            .collect::<Vec<_>>()
            .join(", ")
    }
}
