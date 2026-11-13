use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Markdown,
    Blocks,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct CohereOptions {
    #[serde(default)]
    pub output_format: Option<OutputFormat>,
}
