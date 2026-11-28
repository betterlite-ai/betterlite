use super::error::PassthroughFailure;
use super::model_id::{encoded_model_id, with_model_id};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{
    BedrockEndpointType, RuntimeEndpoint, RuntimeEndpointInput, runtime_endpoint,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use url::Url;

pub(super) fn optional_text(
    params: &Object,
    field: &'static str,
) -> Result<Option<String>, PassthroughFailure> {
    let Some(value) = params.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text = value.as_str().ok_or(PassthroughFailure::Field { field })?;
    Ok(Some(text.to_owned()))
}

// The runtime URL both the plan and the SDK port build: the model_id rewrite over a resolved endpoint
pub(crate) struct RuntimeUrl {
    pub(crate) endpoint: RuntimeEndpoint,
}

impl RuntimeUrl {
    // B658's get_complete_url steps: llm_region_name over the environment, then the runtime endpoint
    pub(crate) fn resolve(
        environment: &PassthroughEnvironment,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<Self, PassthroughFailure> {
        let model_id = optional_text(request.litellm_params, "model_id")?;
        let explicit = optional_text(request.litellm_params, "aws_region_name")?;
        let configured_endpoint =
            optional_text(request.litellm_params, "aws_bedrock_runtime_endpoint")?;
        let region = llm_region_name(LlmRegion {
            explicit: explicit.as_deref(),
            model: Some(request.target.model),
            model_id: model_id.as_deref(),
            environment_region_name: environment.region_name.as_deref(),
            environment_region: environment.region.as_deref(),
            session_region: environment.session_region.as_deref(),
        })?;
        let endpoint = runtime_endpoint(RuntimeEndpointInput {
            api_base: request.api_base,
            configured_endpoint: configured_endpoint.as_deref(),
            environment_endpoint: environment.runtime_endpoint.as_deref(),
            region: &region,
            endpoint_type: BedrockEndpointType::Runtime,
        });
        Ok(Self { endpoint })
    }
}

impl BasePassthroughConfig for RuntimeUrl {
    type Error = PassthroughFailure;
    type Logged = ();

    fn is_streaming_request(&self, endpoint: &str, _request_data: &Object) -> bool {
        endpoint.contains("stream")
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), PassthroughFailure> {
        let model_id = optional_text(request.litellm_params, "model_id")?;
        let rewritten = model_id.as_ref().map_or_else(
            || request.target.endpoint.to_owned(),
            |id| with_model_id(request.target.endpoint, &encoded_model_id(id)),
        );
        let url = self.format_url(
            &rewritten,
            &self.endpoint.endpoint_url,
            request.request_query_params,
        )?;
        Ok((url, self.endpoint.endpoint_url.clone()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassthroughEnvironment {
    pub region_name: Option<String>,
    pub region: Option<String>,
    pub session_region: Option<String>,
    pub runtime_endpoint: Option<String>,
}
