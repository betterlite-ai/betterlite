#[derive(Debug, thiserror::Error)]
pub enum EntryError {
    #[error("litellm_provider of {key} is not a string")]
    ProviderNotText { key: String },
    #[error("the built-in model info for {model} has no string key")]
    BuiltinInfoWithoutKey { model: String },
    #[error("the capability rule info for {model} is not valid JSON")]
    RuleInfoJson {
        model: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RegistrationError<Fetch> {
    #[error("fetching the model cost map from {url} failed")]
    Fetch {
        url: String,
        #[source]
        source: Fetch,
    },
    #[error(transparent)]
    Entry(#[from] EntryError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistrationWarning {
    MissingCachePricing { model: String },
}
