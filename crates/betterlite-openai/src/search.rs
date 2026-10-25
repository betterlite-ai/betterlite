pub mod config;
pub mod records;

pub struct BaseSearchConfig;
pub type CredentialRequest<'request> = config::CredentialRequest<'request>;
pub const PERPLEXITY_UNIFIED_PARAMS: &[&str] = config::PERPLEXITY_UNIFIED_PARAMS;
pub type SearchConfigError = config::SearchConfigError;
pub type SearchError = config::SearchError;
pub type SecretLookup<'source> = config::SecretLookup<'source>;
pub type ListSearchToolsResponse = records::ListSearchToolsResponse;
pub type SearchResponse = records::SearchResponse;
pub type SearchResult = records::SearchResult;
pub type SearchTool = records::SearchTool;
pub type SearchToolInfoResponse = records::SearchToolInfoResponse;
pub type SearchToolLiteLLMParams = records::SearchToolLiteLLMParams;

#[path = "search/search_credential/tests.rs"]
mod search_credential;
#[path = "search/search_hooks/tests.rs"]
mod search_hooks;
#[path = "search/search_types/tests.rs"]
mod search_types;
