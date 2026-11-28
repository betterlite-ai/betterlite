use std::collections::BTreeMap;

use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use betterlite_prompts::arguments::json::{Json, Object};
use percent_encoding::utf8_percent_encode;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_endpoints::model_id::MODEL_ID_SAFE;
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::sources::Credentials;
use crate::credentials::CredentialSource;
use crate::model_routes::ParamMap;
use crate::signing::target::{RegionInputs, TargetRequest, request_target};

use super::ConverseEnvironment;
use super::error::ConverseCallError;

const WRAPPERS: [&str; 3] = ["bedrock/converse/", "bedrock/", "converse/"];
const NOVA_PREFIXES: [&str; 2] = ["nova-2/", "nova/"];
const REGION_FIELD: &str = "aws_region_name";

pub(crate) struct ConverseModelId {
    pub(crate) model_id: String,
    pub(crate) region: Option<String>,
}

// An explicit model_id is encoded as given; otherwise one wrapper comes off a working copy, the first slash segment names a region only from the pinned list, and a nova prefix leaves the id.
pub(crate) fn converse_model_id(model: &str, explicit: Option<&str>) -> ConverseModelId {
    if let Some(model_id) = explicit {
        return ConverseModelId {
            model_id: utf8_percent_encode(model_id, &MODEL_ID_SAFE).to_string(),
            region: None,
        };
    }
    let mut stripped = model;
    for wrapper in WRAPPERS {
        if let Some(rest) = stripped.strip_prefix(wrapper) {
            stripped = rest;
            break;
        }
    }
    let mut model_for_id = model.to_owned();
    let mut region = None;
    if let Some((candidate, rest)) = stripped.split_once('/')
        && AmazonBedrockGlobalConfig
            .get_all_regions()
            .contains(&candidate)
    {
        region = Some(candidate.to_owned());
        stripped = rest;
        rest.clone_into(&mut model_for_id);
    }
    for prefix in NOVA_PREFIXES {
        if stripped.starts_with(prefix) {
            // Python replaces the first occurrence in model-for-id, which still holds the wrapper.
            model_for_id = model_for_id.replacen(prefix, "", 1);
            break;
        }
    }
    ConverseModelId {
        model_id: utf8_percent_encode(&model_for_id, &MODEL_ID_SAFE).to_string(),
        region,
    }
}

pub(crate) struct TargetInput<'text> {
    pub(crate) region_model: &'text str,
    pub(crate) model_id: &'text str,
    pub(crate) stream_path: bool,
    pub(crate) bearer: Option<&'text SecretString>,
    pub(crate) api_base: Option<&'text str>,
    pub(crate) environment: ConverseEnvironment<'text>,
}

pub(crate) struct PreparedTarget {
    pub(crate) region: String,
    pub(crate) credentials: Option<Credentials>,
    pub(crate) url: String,
}

// The region, auth params, credentials and endpoint the call resolves before it transforms the body.
pub(crate) async fn resolve_target<Source>(
    input: &TargetInput<'_>,
    params: &mut ParamMap,
    litellm_params: &mut Object,
    credentials: &Source,
) -> Result<PreparedTarget, ConverseCallError>
where
    Source: CredentialSource,
{
    let mut raw = raw_params(params)?;
    let target = request_target(
        TargetRequest {
            optional_params: &mut raw,
            model: Some(input.region_model),
            region_inputs: RegionInputs {
                environment_region_name: input.environment.region_name,
                environment_region: input.environment.region,
                session_region: input.environment.session_region,
            },
            bearer_token: input.bearer,
        },
        |settings, region| credentials.resolve(settings.clone(), region.to_owned()),
    )
    .await?;
    // The auth params and the endpoint leave the caller's options; everything else keeps its order.
    params.retain(|key, _value| raw.contains_key(key));
    litellm_params.insert(REGION_FIELD.to_owned(), Json::Text(target.region.clone()));
    Ok(PreparedTarget {
        url: endpoint_url(input, target.endpoint.as_deref(), &target.region),
        region: target.region,
        credentials: target.credentials,
    })
}

// No configured proxy endpoint means the runtime endpoint is also the proxy endpoint, and the path is appended without trimming a trailing slash.
fn endpoint_url(
    input: &TargetInput<'_>,
    configured_endpoint: Option<&str>,
    region: &str,
) -> String {
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: input.api_base,
        configured_endpoint,
        environment_endpoint: input.environment.runtime_endpoint,
        region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    append_endpoint(
        &endpoint.proxy_endpoint_url,
        input.model_id,
        input.stream_path,
    )
}

pub(super) fn append_endpoint(endpoint: &str, model_id: &str, stream_path: bool) -> String {
    let suffix = if stream_path {
        "converse-stream"
    } else {
        "converse"
    };
    format!("{endpoint}/model/{model_id}/{suffix}")
}

fn raw_params(params: &ParamMap) -> Result<BTreeMap<String, Box<RawValue>>, ConverseCallError> {
    let mut raw = BTreeMap::new();
    for (name, value) in params {
        raw.insert(
            name.clone(),
            RawValue::from_string(serde_json::to_string(value)?)?,
        );
    }
    Ok(raw)
}
