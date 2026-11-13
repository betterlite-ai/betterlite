use crate::model_urls::base_url::get_vertex_base_url;

use super::error::EndpointTargetError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Partner {
    Llama,
    MistralAi,
    Ai21,
    Claude,
}

pub struct PartnerUrlInput<'text> {
    pub location: &'text str,
    pub project: &'text str,
    pub partner: Partner,
    pub stream: bool,
    pub model: &'text str,
    pub api_base: Option<&'text str>,
}

pub fn create_vertex_url(input: &PartnerUrlInput<'_>) -> Result<String, EndpointTargetError> {
    let base = match input.api_base {
        Some(base) => base.to_owned(),
        None => get_vertex_base_url(Some(input.location))?,
    };
    let project = input.project;
    let location = input.location;
    let model = input.model;
    let (version, publisher) = match input.partner {
        Partner::Llama => {
            return Ok(format!(
                "{base}/v1/projects/{project}/locations/{location}/endpoints/openapi/chat/completions"
            ));
        }
        Partner::MistralAi => ("v1", "mistralai"),
        Partner::Ai21 => ("v1beta1", "ai21"),
        Partner::Claude => ("v1", "anthropic"),
    };
    let endpoint = if input.stream {
        "streamRawPredict"
    } else {
        "rawPredict"
    };
    Ok(format!(
        "{base}/{version}/projects/{project}/locations/{location}/publishers/{publisher}/models/{model}:{endpoint}"
    ))
}
