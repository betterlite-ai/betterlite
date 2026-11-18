use betterlite_bedrock::legacy_models::family::{ClassAttributes, Family};
use betterlite_config::sections::bedrock_legacy_models::families::{
    Ai21Values, AnthropicValues, CohereValues, LlamaValues, MistralValues, TitanValues,
};
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    Mappings { cases: Vec<MapCase> },
    Constructors(ConstructorCases),
    Projection(ProjectionCase),
    OutputText(OutputCase),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapCase {
    pub(crate) family: Family,
    // the family's map_openai_params arguments, spelled as the OpenAI request spells them
    pub(crate) non_default: Object,
    #[serde(default)]
    pub(crate) optional: Object,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) drop_params: bool,
    pub(crate) global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorCases {
    pub(crate) default_max_tokens: i64,
    pub(crate) cases: Vec<ConstructorRow>,
}

// Each row is one constructor call sequence; the family tag names the class Python builds.
#[derive(Debug, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub(crate) enum ConstructorRow {
    Ai21 {
        #[serde(default)]
        calls: Vec<Ai21Values>,
    },
    Cohere {
        #[serde(default)]
        calls: Vec<CohereValues>,
    },
    Llama {
        #[serde(default)]
        calls: Vec<LlamaValues>,
    },
    Mistral {
        #[serde(default)]
        calls: Vec<MistralValues>,
    },
    Titan {
        #[serde(default)]
        calls: Vec<TitanValues>,
    },
    Anthropic {
        #[serde(default)]
        calls: Vec<AnthropicValues>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectionCase {
    pub(crate) family: Family,
    // the Python fixture class dictionary this case mirrors
    pub(crate) attributes: ClassAttributes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OutputCase {
    pub(crate) response: Object,
}
