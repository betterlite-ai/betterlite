use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite LiteLLM pin",
    description = "The LiteLLM release betterlite reimplements, which of its paths count as product, and which of those are left out on purpose."
)]
pub(crate) struct Litellm {
    #[schemars(length(min = 1))]
    pub(crate) repository: String,
    #[schemars(length(min = 1))]
    pub(crate) version: String,
    #[schemars(
        regex(pattern = "^[0-9a-f]{40}$"),
        description = "The commit the oracle and every LiteLLM link pin."
    )]
    pub(crate) commit: String,
    #[schemars(
        description = "Paths in the LiteLLM repository that make up the product. A directory ends with a slash."
    )]
    pub(crate) roots: Vec<String>,
    #[schemars(
        description = "Paths under the roots that betterlite deliberately does not reimplement."
    )]
    pub(crate) excluded: Vec<Exclusion>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Exclusion {
    #[schemars(
        length(min = 1),
        description = "Glob over LiteLLM paths. `*` stays inside one directory, `**` crosses directories."
    )]
    pub(crate) glob: String,
    #[schemars(length(min = 1))]
    pub(crate) why: String,
}
