use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_prompts::arguments::json::{Json as BodyJson, Object};
use indexmap::IndexMap;
use serde_json::Number;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::messages::transform::{
    MessagesRequest, TransformSources, transform_request as transform_bedrock_request,
};

use super::MANTLE_PREFIX;
use super::error::MantleMessagesError;

const ANTHROPIC_BETA_BODY_FIELD: &str = "anthropic_beta";
const ANTHROPIC_BETA_HEADER: &str = "anthropic-beta";
const ANTHROPIC_VERSION_BODY_FIELD: &str = "anthropic_version";
const MODEL_FIELD: &str = "model";
const STREAM_FIELD: &str = "stream";

// AmazonMantleMessagesConfig.transform_anthropic_messages_request: the parent prepares the request for the model ID,
// the beta list moves from the body into the signed headers, and the model and a literal stream=True come back.
pub async fn transform_request<Source, Transport, Lookup>(
    request: &MessagesRequest<'_>,
    headers: &mut IndexMap<String, String>,
    sources: &mut TransformSources<'_, '_, Source, Transport, Lookup>,
) -> Result<Object, MantleMessagesError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    let model_id = request.model.replacen(MANTLE_PREFIX, "", 1);
    let stripped = MessagesRequest {
        model: &model_id,
        messages: request.messages.clone(),
        optional_params: request.optional_params.clone(),
        headers: request.headers,
        model_cost: request.model_cost,
        drop_params: request.drop_params,
    };
    let mut body = transform_bedrock_request(&stripped, sources).await?;
    move_betas_into_header(&body, headers)?;
    restore_messages_fields(
        &mut body,
        &model_id,
        request.optional_params.get(STREAM_FIELD),
    );
    Ok(body)
}

// The body keeps neither the version nor the beta list, gets the model back only when it has one, and streams
// only for a literal true.
pub(super) fn restore_messages_fields(
    body: &mut Object,
    model_id: &str,
    stream: Option<&BodyJson>,
) {
    body.shift_remove(ANTHROPIC_VERSION_BODY_FIELD);
    body.shift_remove(ANTHROPIC_BETA_BODY_FIELD);
    if !model_id.is_empty() {
        body.insert(MODEL_FIELD.to_owned(), BodyJson::Text(model_id.to_owned()));
    }
    if matches!(stream, Some(BodyJson::Bool(true))) {
        body.insert(STREAM_FIELD.to_owned(), BodyJson::Bool(true));
    }
}

// _move_betas_into_header: a nonempty list joins into the exact lowercase key; an empty one removes only that key,
// so a differently cased caller key stays as it was.
pub(super) fn move_betas_into_header(
    body: &Object,
    headers: &mut IndexMap<String, String>,
) -> Result<(), MantleMessagesError> {
    let betas = declared_betas(body.get(ANTHROPIC_BETA_BODY_FIELD))?;
    if betas.is_empty() {
        headers.shift_remove(ANTHROPIC_BETA_HEADER);
        return Ok(());
    }
    headers.insert(ANTHROPIC_BETA_HEADER.to_owned(), betas.join(","));
    Ok(())
}

// request.anthropic_beta or (): every falsey value counts as no betas, and a truthy non-list fails the shape check.
fn declared_betas(value: Option<&BodyJson>) -> Result<Vec<String>, MantleMessagesError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        BodyJson::List(items) => {
            let mut betas = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let BodyJson::Text(beta) = item else {
                    return Err(MantleMessagesError::BetaItem { index });
                };
                betas.push(beta.clone());
            }
            Ok(betas)
        }
        BodyJson::Null | BodyJson::Bool(false) => Ok(Vec::new()),
        BodyJson::Number(number) if is_zero_number(number) => Ok(Vec::new()),
        BodyJson::Text(text) if text.is_empty() => Ok(Vec::new()),
        BodyJson::Object(fields) if fields.is_empty() => Ok(Vec::new()),
        BodyJson::Bool(true) | BodyJson::Number(_) | BodyJson::Text(_) | BodyJson::Object(_) => {
            Err(MantleMessagesError::BetaNotList)
        }
    }
}

// Python's `if value:` reads both signed zeros as falsey, and every JSON number that parses to ±0.0 is one.
pub(super) fn is_zero_number(number: &Number) -> bool {
    const MAGNITUDE_MASK: u64 = 0x7fff_ffff_ffff_ffff;
    number
        .as_f64()
        .is_some_and(|value| value.to_bits() & MAGNITUDE_MASK == 0)
}
