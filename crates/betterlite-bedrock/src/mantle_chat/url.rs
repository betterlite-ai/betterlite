use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::bedrock_model_capabilities::mantle::mantle_messages_url;
use crate::invoke_primitives::config::Params;
use crate::invoke_primitives::url::UrlError;
use crate::mantle_inputs::records::MantleGlobals;

const REGION_FIELD: &str = "aws_region_name";
const ENDPOINT_FIELD: &str = "aws_bedrock_runtime_endpoint";

// AmazonMantleConfig.get_complete_url and AmazonMantleMessagesConfig.get_complete_url: B591 resolves the region from
// the original model and options, then B690 builds the endpoint from api_base, the configured endpoint and the region.
pub fn complete_url(
    api_base: Option<&str>,
    model: &str,
    optional_params: &Params,
    globals: &MantleGlobals,
) -> Result<String, UrlError> {
    let explicit = read_string(optional_params, REGION_FIELD)?;
    let region = llm_region_name(LlmRegion {
        explicit: explicit.as_deref(),
        model: Some(model),
        model_id: None,
        environment_region_name: globals.aws_region_name.as_deref(),
        environment_region: globals.aws_region.as_deref(),
        session_region: None,
    })?;
    let configured_endpoint = read_string(optional_params, ENDPOINT_FIELD)?;
    Ok(mantle_messages_url(
        api_base,
        configured_endpoint.as_deref(),
        globals.mantle_api_base.as_deref(),
        &region,
    ))
}

// The invoke configs take Box<RawValue>; a value of the wrong type names the field it came from.
fn read_string(params: &Params, field: &'static str) -> Result<Option<String>, UrlError> {
    params.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str::<Option<String>>(raw.get())
            .map_err(|source| UrlError::Field { field, source })
    })
}
