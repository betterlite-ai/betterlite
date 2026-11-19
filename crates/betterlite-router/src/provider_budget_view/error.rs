use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProviderBudgetViewError {
    // the cache adapter logs its driver's failure; the view only reports the key it asked for
    #[error("the provider spend cached under {key} could not be read")]
    Spend { key: String },
    #[error("the provider budget expiry cached under {key} is not a finite count of seconds")]
    Expiry { key: String },
}
