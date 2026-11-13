use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::response_params::bundled::BundledReasoningModels;
use betterlite_openai::response_params::capabilities::{
    CUSTOM_LLM_PROVIDER, provider_capabilities,
};
// ast-grep-ignore: no-long-names
use betterlite_openai::response_params::decode::decode_response_capability_error as decode_body;
use betterlite_openai::response_params::map_openai_params;
use betterlite_openai::response_params::params::supported_openai_params;
use serde_json::{Value, json};

use super::fixture::Fixture;
use super::{Call, Probe, Probes};
use crate::helpers::HelperResult;

pub(super) async fn run(call: &Call) -> HelperResult {
    let mut bundled = BundledReasoningModels::default();
    match call {
        Call::MinimumOutputTokens(probes)
        | Call::ReasoningRejected(probes)
        | Call::Gpt5TemperatureDefaultEffort(probes)
        | Call::TopPWithReasoning(probes)
        | Call::UnknownCatalogModel(probes)
        | Call::ReasoningSummaryWithoutEffort(probes) => map_probes(probes, &mut bundled).await,
        Call::ResponsesParameterSet(probes) => Ok(supported_probes(probes)),
        Call::ProviderCapabilities(probes) => capability_probes(probes),
    }
}

async fn map_probes(probes: &Probes, bundled: &mut BundledReasoningModels) -> HelperResult {
    let mut results = Vec::with_capacity(probes.probes.len());
    for probe in &probes.probes {
        let mut fixture = Fixture::new(&probe.model_cost);
        let drop_params = DropParams {
            global: probe.global_drop_params,
            request: probe.drop_params,
        };
        let mapped = map_openai_params(&mut fixture, &probe.options(), drop_params, bundled).await;
        results.push(match mapped {
            Ok(params) => serde_json::to_value(params)?,
            Err(error) => json!({"error": error.status_code()}),
        });
    }
    Ok(Value::Array(results))
}

fn supported_probes(probes: &Probes) -> Value {
    let supported = supported_openai_params().into_iter().collect::<Vec<_>>();
    let mut results = Vec::with_capacity(probes.probes.len());
    for _probe in &probes.probes {
        results.push(json!({"supported": supported}));
    }
    Value::Array(results)
}

fn capability_probes(probes: &Probes) -> HelperResult {
    let mut results = Vec::with_capacity(probes.probes.len());
    for probe in &probes.probes {
        results.push(capability_value(probe)?);
    }
    Ok(Value::Array(results))
}

fn capability_value(probe: &Probe) -> HelperResult {
    let flags = provider_capabilities(probe.provider.unwrap_or(CUSTOM_LLM_PROVIDER));
    let mut value = json!({
        "custom_llm_provider": flags.custom_llm_provider,
        "supports_native_file_search": flags.supports_native_file_search,
        "supports_encrypted_agent_messages": flags.encrypted_agent_messages,
    });
    if let Some(body) = &probe.body {
        let bytes = serde_json::to_vec(body)?;
        let decoded = decode_body(&bytes).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: String::from("ResponseDecodeError"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let decoded_body = Value::Object(decoded);
        if let Value::Object(fields) = &mut value {
            fields.insert(String::from("json_object_body"), decoded_body.clone());
            fields.insert(String::from("delete_response_body"), decoded_body);
        }
    }
    Ok(value)
}
