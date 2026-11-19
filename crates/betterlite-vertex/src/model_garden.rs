use crate::model_urls::base_url::get_vertex_base_url;
use crate::model_urls::error::VertexUrlError;
use crate::model_urls::model_name::get_vertex_base_model_name;

mod tests;

pub const DEFAULT_LOCATION: &str = "us-central1";

// the base the OpenAI-compatible call is sent to and the model the body carries; a deployed endpoint sends an empty model
pub struct ModelGardenTarget {
    pub api_base: String,
    pub model: String,
}

pub struct ModelGardenTargetInput<'input> {
    pub model: &'input str,
    pub api_base: Option<&'input str>,
    pub vertex_location: Option<&'input str>,
    pub vertex_project: Option<&'input str>,
    // the project B1302's authentication resolved when the caller requested none
    pub resolved_project: &'input str,
}

pub fn model_garden_target(
    input: &ModelGardenTargetInput<'_>,
) -> Result<ModelGardenTarget, VertexUrlError> {
    let model = get_vertex_base_model_name(input.model);
    let api_base = match input.api_base {
        Some(api_base) => api_base.to_owned(),
        None => get_model_garden_url(
            input.vertex_location.unwrap_or(DEFAULT_LOCATION),
            input.vertex_project.unwrap_or(input.resolved_project),
            model,
        )?,
    };
    Ok(ModelGardenTarget {
        api_base,
        model: if model.contains('/') {
            model.to_owned()
        } else {
            String::new()
        },
    })
}

// a publisher model id travels in the JSON body on the shared OpenAPI route; a deployed endpoint id travels in the path
pub fn get_model_garden_url(
    location: &str,
    project: &str,
    model: &str,
) -> Result<String, VertexUrlError> {
    let base_url = get_vertex_base_url(Some(location))?;
    let path = if model.contains('/') {
        format!("v1/projects/{project}/locations/{location}/endpoints/openapi")
    } else {
        format!("v1beta1/projects/{project}/locations/{location}/endpoints/{model}")
    };
    Ok(format!("{base_url}/{path}"))
}
