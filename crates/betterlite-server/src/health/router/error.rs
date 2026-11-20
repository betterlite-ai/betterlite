use betterlite_router::model_catalog_lookup::error::LookupError;

// The ways a router health verdict fails: the catalog lookup a dependency resolves through, which LiteLLM raises out of get_model_list.
#[derive(Debug, thiserror::Error)]
pub enum RouterHealthError {
    #[error("the model catalog lookup failed")]
    Lookup(#[from] LookupError),
}
