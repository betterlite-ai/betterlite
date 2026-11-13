mod fixture;
mod input;
mod tests;
mod transport;

use std::future::Future;

use betterlite_models::model_catalog_keys::{ModelCostKeys, PotentialModelNames};
use betterlite_models::model_metadata_resolution::error::ModelMetadataError;
use betterlite_models::model_metadata_resolution::hf_config::HfConfigClient;
use betterlite_models::model_metadata_resolution::names::{
    PotentialNamesInput, potential_model_names,
};
use betterlite_models::model_metadata_resolution::{
    MetadataEnvironment, MetadataRequest, get_model_info_helper,
};
use secrecy::SecretString;
use serde_json::value::RawValue;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};
use fixture::{FixtureContext, FixtureError, NORMALIZERS};
use input::{Input, Query, Scenario};
use transport::{ScriptedDynamic, ScriptedHf};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let context = FixtureContext::load(&input.fixture).map_err(|error| failure(&error))?;
        let observations = input
            .queries
            .iter()
            .map(|query| observe(input.scenario, &context, query))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(json!(observations))
    }
}

fn failure(error: &FixtureError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn observe(
    scenario: Scenario,
    context: &FixtureContext,
    query: &Query,
) -> Result<Value, Box<HelperError>> {
    if scenario == Scenario::PotentialNames {
        return Ok(observe_names(context, query));
    }
    observe_metadata(context, query)
}

fn observe_names(context: &FixtureContext, query: &Query) -> Value {
    let resolution = context.resolution();
    let names = potential_model_names(&PotentialNamesInput {
        model: &query.model,
        custom_llm_provider: query.custom_llm_provider.as_deref(),
        resolution: &resolution,
        normalizers: NORMALIZERS,
    });
    json!({"result": names_json(&names)})
}

fn observe_metadata(context: &FixtureContext, query: &Query) -> Result<Value, Box<HelperError>> {
    let dynamic = ScriptedDynamic::new(query.dynamic.as_ref());
    let hf = ScriptedHf::new(query.hf.as_ref());
    let api_key = query.api_key.clone().map(SecretString::from);
    let request = MetadataRequest {
        model: &query.model,
        custom_llm_provider: query.custom_llm_provider.as_deref(),
        api_base: query.api_base.as_deref(),
        api_key: api_key.as_ref(),
    };
    let generalizations = context.generalizations().map_err(|error| failure(&error))?;
    let resolution = context.resolution();
    let environment = MetadataEnvironment {
        model_cost: &context.map.entries,
        vertex: &context.map.vertex,
        generalizations: &generalizations,
        resolution: &resolution,
        normalizers: NORMALIZERS,
    };
    let mut keys = ModelCostKeys::default();
    let answer = block_on(get_model_info_helper(
        &request,
        &environment,
        &mut keys,
        &dynamic,
        &HfConfigClient {
            client: &hf,
            base_url: "https://huggingface.co".to_owned(),
        },
    ))?;
    Ok(observation(answer, &dynamic, &hf))
}

fn observation(
    answer: Result<Box<RawValue>, ModelMetadataError>,
    dynamic: &ScriptedDynamic,
    hf: &ScriptedHf,
) -> Value {
    answer.map_or_else(
        |error| {
            json!({
                "raised": match &error {
                    ModelMetadataError::NotMapped { .. } => "not_mapped",
                    ModelMetadataError::Failed { .. } => "failed",
                },
                "message": error.to_string(),
                "dynamic_calls": dynamic.calls(),
                "hf_requests": hf.requests(),
            })
        },
        |raw| {
            json!({
                "result": raw,
                "dynamic_calls": dynamic.calls(),
                "hf_requests": hf.requests(),
            })
        },
    )
}

fn block_on<FutureType>(future: FutureType) -> Result<FutureType::Output, Box<HelperError>>
where
    FutureType: Future,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| failure(&FixtureError::Runtime { source }))?;
    Ok(runtime.block_on(future))
}

fn names_json(names: &PotentialModelNames) -> Value {
    json!({
        "split_model": names.split_model,
        "combined_model_name": names.combined_model_name,
        "stripped_model_name": names.stripped_model_name,
        "combined_stripped_model_name": names.combined_stripped_model_name,
        "provider_prefixed_model_name": names.provider_prefixed_model_name,
        "custom_llm_provider": names.custom_llm_provider,
    })
}
