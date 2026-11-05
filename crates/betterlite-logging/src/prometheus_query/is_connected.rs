use super::PrometheusSettings;
use super::error::QueryError;

impl PrometheusSettings {
    pub fn endpoint<Failure>(&self) -> Result<&str, QueryError<Failure>> {
        let Some(url) = self.url.as_deref().filter(|url| !url.is_empty()) else {
            return Err(QueryError::NotConfigured);
        };
        Ok(url)
    }
}

mod tests;
