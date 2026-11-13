use super::options::{CohereOptions, OutputFormat};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CohereRequest {
    pub model: String,
    pub document: CohereParseDocument,
    pub output_format: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum CohereParseDocument {
    #[serde(rename = "image_url")]
    ImageUrl { image_url: String },
}

pub(crate) fn build_request(
    model: &str,
    image_url: String,
    params: &CohereOptions,
) -> CohereRequest {
    CohereRequest {
        model: model.to_owned(),
        document: CohereParseDocument::ImageUrl { image_url },
        output_format: match params.output_format.unwrap_or_default() {
            OutputFormat::Markdown => "markdown",
            OutputFormat::Blocks => "blocks",
        }
        .to_owned(),
    }
}
