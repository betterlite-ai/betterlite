use betterlite_openai::chatgpt::chat::normalizer::ChatGptToolCallNormalizer;
use betterlite_openai::chatgpt::chat::{Authenticator as _, ChatGptConfig};
use betterlite_openai::chatgpt::headers::PlatformEnvironment;
use betterlite_openai::chatgpt::headers::session::LitellmParamsInput;
use betterlite_openai::openai_base_config::{CatalogCapabilities, Object};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::{StreamExt as _, stream};
use secrecy::ExposeSecret as _;
use serde_json::Value;

use super::fixture::{FixedSession, FixtureError, Login, Recording};
use super::{HeadersInput, Input, NormalizeInput, ParamsInput, ProviderInfoInput, Step};

fn decode_chunks(chunks: &[Value]) -> Result<Vec<Chunk>, FixtureError> {
    chunks
        .iter()
        .map(|value| serde_json::from_value(value.clone()).map_err(FixtureError::Json))
        .collect()
}

async fn collect_async(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut source = ChatGptToolCallNormalizer::from(stream::iter(chunks));
    let mut output = Vec::new();
    while let Some(chunk) = source.next().await {
        output.push(chunk);
    }
    output
}

fn project(chunks: &[Chunk]) -> Result<Value, FixtureError> {
    let values = chunks
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(values))
}

fn provider_value(
    config: &mut ChatGptConfig<Recording<Login>>,
    spec: &ProviderInfoInput,
) -> Result<Object, FixtureError> {
    let api_base = config.authenticator.api_base().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let info = runtime.block_on(config.provider_info(
        &spec.model,
        spec.api_base.as_deref(),
        spec.api_key.as_deref(),
        &spec.provider,
    ))?;
    let mut output = Object::new();
    output.insert(String::from("api_base"), Value::String(api_base));
    output.insert(
        String::from("provider_api_base"),
        Value::String(info.api_base),
    );
    output.insert(
        String::from("api_key"),
        Value::String(info.api_key.expose_secret().to_owned()),
    );
    output.insert(String::from("provider"), Value::String(info.provider));
    Ok(output)
}

fn headers_value(
    config: &mut ChatGptConfig<Recording<Login>>,
    platform: &dyn PlatformEnvironment,
    spec: &HeadersInput,
) -> Object {
    let params = LitellmParamsInput::Mapping(spec.litellm_params.clone());
    let mut output = Object::new();
    for (name, value) in config.validate_environment(&params, platform, &FixedSession) {
        output.insert(name, Value::String(value));
    }
    output
}

fn params_value(config: &ChatGptConfig<Recording<Login>>, spec: &ParamsInput) -> Object {
    let catalog = CatalogCapabilities {
        chat_models: spec.chat_models.clone(),
        text_models: spec.text_models.clone(),
    };
    let mut mapped = Vec::new();
    for requested in &spec.requests {
        let mut optional = Object::new();
        config.map_openai_params(&spec.model, requested, &mut optional, &catalog);
        mapped.push(Value::Object(optional));
    }
    let mut output = Object::new();
    output.insert(String::from("params"), Value::Array(mapped));
    output
}

fn normalize_value(spec: &NormalizeInput) -> Result<Object, FixtureError> {
    let sync = ChatGptToolCallNormalizer::from(decode_chunks(&spec.chunks)?.into_iter())
        .collect::<Vec<_>>();
    let runtime = tokio::runtime::Builder::new_current_thread().build()?;
    let asynchronous = runtime.block_on(collect_async(decode_chunks(&spec.chunks)?));
    let mut output = Object::new();
    output.insert(String::from("sync"), project(&sync)?);
    output.insert(String::from("async"), project(&asynchronous)?);
    Ok(output)
}

pub(super) fn step_value(
    input: &Input,
    config: &mut ChatGptConfig<Recording<Login>>,
) -> Result<Object, FixtureError> {
    Ok(match &input.step {
        Step::ProviderInfo(spec) => provider_value(config, spec)?,
        Step::Headers(spec) => headers_value(config, &input.platform, spec),
        Step::Params(spec) => params_value(config, spec),
        Step::Normalize(spec) => normalize_value(spec)?,
    })
}
