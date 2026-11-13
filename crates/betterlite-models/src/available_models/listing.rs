use std::error::Error;

use secrecy::SecretString;

use crate::provider_metadata_factory::selection::ProviderModelMetadata;

#[derive(Clone, Copy, Debug)]
pub struct ListingCredentials<'credentials> {
    pub api_key: Option<&'credentials SecretString>,
    pub api_base: Option<&'credentials str>,
}

pub trait ModelListing {
    type Error: Error;

    fn get_models(
        &self,
        adapter: &ProviderModelMetadata,
        credentials: ListingCredentials<'_>,
    ) -> Result<Vec<String>, Self::Error>;
}
