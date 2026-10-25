pub mod config;
pub mod credentials;
pub mod provider;
pub mod records;

pub use config::{BaseSearchConfig, CredentialRequest, SearchConfigError, SecretLookup};
pub use records::{
    ListSearchToolsResponse, SearchResponse, SearchResult, SearchTool, SearchToolInfoResponse,
    SearchToolLiteLLMParams,
};

#[path = "search/credentials/tests.rs"]
mod search_credential;
#[path = "search/provider/tests.rs"]
mod search_hooks;
#[path = "search/records/tests.rs"]
mod search_types;
