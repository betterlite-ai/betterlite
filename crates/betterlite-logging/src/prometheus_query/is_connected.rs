use super::PrometheusSettings;
use super::error::QueryError;
use betterlite_config::loader::EnvironmentLookup;

impl PrometheusSettings {
    #[must_use]
    pub fn from_lookup(lookup: &dyn EnvironmentLookup) -> Self {
        Self {
            url: lookup
                .lookup("PROMETHEUS_URL")
                .filter(|url| !url.is_empty()),
            selected_instance: lookup
                .lookup("PROMETHEUS_SELECTED_INSTANCE")
                .filter(|instance| !instance.is_empty()),
        }
    }

    pub fn endpoint<Failure>(&self) -> Result<&str, QueryError<Failure>> {
        let Some(url) = self.url.as_deref().filter(|url| !url.is_empty()) else {
            return Err(QueryError::NotConfigured);
        };
        Ok(url)
    }
}

mod tests;
