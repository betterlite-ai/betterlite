use betterlite_models::catalog::ModelCatalog;
use chrono::NaiveDate;

pub struct CatalogDeprecation {
    pub deprecation_date: NaiveDate,
    pub litellm_provider: Option<String>,
}

pub trait DeprecationCatalog {
    fn deprecation(&self, model_key: &str) -> Option<CatalogDeprecation>;
}

impl DeprecationCatalog for ModelCatalog {
    fn deprecation(&self, model_key: &str) -> Option<CatalogDeprecation> {
        let entry = self.entries.get(model_key)?;
        Some(CatalogDeprecation {
            deprecation_date: entry.fields.metadata.deprecation_date?,
            litellm_provider: Some(entry.fields.metadata.litellm_provider.clone()),
        })
    }
}
