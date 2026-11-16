use crate::loader::EnvironmentLookup;

mod tests;

pub const VERSION_VARIABLE: &str = "AZURE_DEFAULT_RESPONSES_API_VERSION";
pub const DEFAULT_VERSION: &str = "preview";

// zero-size on purpose: Sections sits at the loader's stack limit and in clippy's large_futures limit (B9972)
crate::sections::stub_section!();

// LiteLLM reads the variable with os.getenv; the project's compatibility rules make an empty value unset
pub fn default_responses_api_version<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    match lookup.lookup(VERSION_VARIABLE) {
        Some(value) if !value.is_empty() => value,
        Some(_) | None => DEFAULT_VERSION.to_owned(),
    }
}
