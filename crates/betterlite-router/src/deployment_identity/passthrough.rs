use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::passthrough::transformation::replace_path_segment;

use super::error::InvalidDeployment;

// get_llm_provider(model, custom_llm_provider, api_base): the model without its provider prefix, none when it fails.
pub type StripProvider = fn(&str, Option<&str>, Option<&str>) -> Option<String>;

pub struct PassThroughCall<'call> {
    pub endpoint: Option<&'call str>,
    pub custom_llm_provider: Option<&'call str>,
    pub api_base: Option<&'call str>,
    // the model name the caller put in the endpoint
    pub model: &'call str,
    // the deployment's own model name
    pub model_name: &'call str,
}

// The endpoint with the caller's model swapped for the deployment's, or none when the endpoint is unset or empty.
#[must_use]
pub fn endpoint_for_deployment(call: &PassThroughCall<'_>, strip: StripProvider) -> Option<String> {
    let endpoint = call.endpoint.filter(|endpoint| !endpoint.is_empty())?;
    let replacement = strip(call.model_name, call.custom_llm_provider, call.api_base)
        .unwrap_or_else(|| call.model_name.to_owned());
    Some(replace_path_segment(endpoint, call.model, &replacement))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VertexPassThrough {
    pub project: String,
    pub location: String,
    pub credentials: Option<Json>,
}

// A named credential's value wins over the deployment's own litellm_params.
fn credential_or_param<'maps>(
    credential_values: &'maps JsonObject,
    litellm_params: &'maps JsonObject,
    field: &str,
) -> Option<&'maps Json> {
    let from_credential = credential_values
        .get(field)
        .filter(|value| !matches!(value, Json::Null));
    from_credential.or_else(|| {
        litellm_params
            .get(field)
            .filter(|value| !matches!(value, Json::Null))
    })
}

// The vertex project, location and credentials to register, or none unless use_in_pass_through is true on a vertex_ai deployment.
pub fn vertex_pass_through(
    litellm_params: &JsonObject,
    credential_values: &JsonObject,
    custom_llm_provider: &str,
) -> Result<Option<VertexPassThrough>, InvalidDeployment> {
    if !matches!(
        litellm_params.get("use_in_pass_through"),
        Some(Json::Bool(true))
    ) || custom_llm_provider != "vertex_ai"
    {
        return Ok(None);
    }
    let chosen_project = credential_or_param(credential_values, litellm_params, "vertex_project");
    let chosen_location = credential_or_param(credential_values, litellm_params, "vertex_location");
    let (Some(given_project), Some(given_location)) = (chosen_project, chosen_location) else {
        return Err(InvalidDeployment::VertexLocation);
    };
    let (Json::Text(project), Json::Text(location)) = (given_project, given_location) else {
        return Err(InvalidDeployment::NotText {
            field: "vertex_project and vertex_location",
        });
    };
    Ok(Some(VertexPassThrough {
        project: project.clone(),
        location: location.clone(),
        credentials: credential_or_param(credential_values, litellm_params, "vertex_credentials")
            .cloned(),
    }))
}
