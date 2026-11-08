pub mod error;
pub mod headers;
pub mod mapping;
pub mod request;
pub mod url;

mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HuggingfaceEnv {
    pub hf_api_base: Option<String>,
    pub huggingface_api_base: Option<String>,
}
