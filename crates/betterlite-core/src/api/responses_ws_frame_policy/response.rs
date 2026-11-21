use std::borrow::Cow;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::PresidioGuardrailCallback;
use indexmap::IndexMap;

use super::{
    DELTA_EVENT_TYPES, FramePolicy, FramePolicyError, PiiCall, RequestData, UnmaskPiiText,
    decode_object, encode,
};
use crate::api::responses_stream_events::JsonShape;

// only the first guardrail's unmask function runs, with the tokens the request masking stored
pub fn unmask_response_event<'message, Guard>(
    policy: &FramePolicy<'_, Guard>,
    request_data: &RequestData,
    message: &'message str,
) -> Result<Cow<'message, str>, FramePolicyError<Guard::Error>>
where
    Guard: PresidioGuardrailCallback,
    Guard::Error: 'static,
{
    let Some(first) = policy.guardrail_callbacks.first() else {
        return Ok(Cow::Borrowed(message));
    };
    let pii_tokens = stored_pii_tokens(request_data);
    if pii_tokens.is_empty() {
        return Ok(Cow::Borrowed(message));
    }
    let Some(mut event) = decode_object(message)? else {
        return Ok(Cow::Borrowed(message));
    };
    let unmask = first.unmask_pii_text;
    let kind = event.get("type").and_then(JsonShape::optional_str);
    let completed = kind == Some("response.completed");
    let streamed_delta = kind.is_some_and(|delta_kind| DELTA_EVENT_TYPES.contains(&delta_kind));
    if completed {
        let modified = unmask_completed(&mut event, unmask, &pii_tokens);
        return if modified {
            encode(event)
        } else {
            Ok(Cow::Borrowed(message))
        };
    }
    if streamed_delta && let Some(Json::Text(delta)) = event.get_mut("delta") {
        let unmasked = unmask(delta, &pii_tokens);
        if unmasked != *delta {
            *delta = unmasked;
            return encode(event);
        }
    }
    Ok(Cow::Borrowed(message))
}

// a metadata or token map of any other shape counts as no tokens at all
fn stored_pii_tokens(request_data: &RequestData) -> IndexMap<String, String> {
    let Some(metadata) = request_data.get("metadata") else {
        return IndexMap::new();
    };
    let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(metadata.get()) else {
        return IndexMap::new();
    };
    let Some(Json::Object(tokens)) = fields.get("pii_tokens") else {
        return IndexMap::new();
    };
    tokens
        .iter()
        .map(|(token, original)| {
            original
                .optional_str()
                .map(|text| (token.clone(), text.to_owned()))
        })
        .collect::<Option<IndexMap<String, String>>>()
        .unwrap_or_default()
}

fn unmask_completed(
    event: &mut IndexMap<String, Json>,
    unmask: &UnmaskPiiText,
    pii_tokens: &IndexMap<String, String>,
) -> bool {
    let Some(Json::Object(response)) = event.get_mut("response") else {
        return false;
    };
    let Some(Json::List(items)) = response.get_mut("output") else {
        return false;
    };
    let mut modified = false;
    for item in items {
        if let Json::Object(fields) = item
            && let Some(Json::List(blocks)) = fields.get_mut("content")
        {
            let changed = unmask_blocks(blocks, unmask, pii_tokens);
            modified = changed || modified;
        }
    }
    modified
}

fn unmask_blocks(
    blocks: &mut [Json],
    unmask: &UnmaskPiiText,
    pii_tokens: &IndexMap<String, String>,
) -> bool {
    let mut modified = false;
    for block in blocks {
        if let Json::Object(fields) = block
            && let Some(Json::Text(text)) = fields.get_mut("text")
        {
            let unmasked = unmask(text, pii_tokens);
            if unmasked != *text {
                *text = unmasked;
                modified = true;
            }
        }
    }
    modified
}

// the relay already dropped the delta and done events, so response.completed is the one copy masked here
pub async fn mask_response_completed<'message, Guard>(
    policy: &FramePolicy<'_, Guard>,
    request_data: &RequestData,
    message: &'message str,
) -> Result<Cow<'message, str>, FramePolicyError<Guard::Error>>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Send + Sync,
    Guard::Error: 'static,
{
    if policy.output_guardrail_callbacks.is_empty() {
        return Ok(Cow::Borrowed(message));
    }
    let Some(mut event) = decode_object(message)? else {
        return Ok(Cow::Borrowed(message));
    };
    if !matches!(event.get("type"), Some(Json::Text(kind)) if kind == "response.completed") {
        return Ok(Cow::Borrowed(message));
    }
    let mut modified = false;
    for guard in policy.output_guardrail_callbacks {
        let settings = guard.presidio_settings(request_data);
        let call = PiiCall {
            guard,
            settings: settings.as_ref(),
            request_data,
            output_parse_pii: false,
        };
        if let Some(Json::Object(response)) = event.get_mut("response")
            && let Some(Json::List(items)) = response.get_mut("output")
        {
            let changed = mask_output_items(&call, items)
                .await
                .map_err(FramePolicyError::Guardrail)?;
            modified = changed || modified;
        }
    }
    if modified {
        encode(event)
    } else {
        Ok(Cow::Borrowed(message))
    }
}

async fn mask_output_items<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    items: &mut [Json],
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let mut modified = false;
    for item in items {
        let Json::Object(fields) = item else {
            continue;
        };
        if let Some(Json::Text(arguments)) = fields.get_mut("arguments") {
            let changed = mask_changed_text(call, arguments).await?;
            modified = changed || modified;
        }
        for blocks_field in ["summary", "content"] {
            if let Some(Json::List(blocks)) = fields.get_mut(blocks_field) {
                let changed = mask_output_blocks(call, blocks).await?;
                modified = changed || modified;
            }
        }
    }
    Ok(modified)
}

async fn mask_output_blocks<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    blocks: &mut [Json],
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let mut modified = false;
    for block in blocks {
        if let Json::Object(fields) = block
            && let Some(Json::Text(text)) = fields.get_mut("text")
        {
            let changed = mask_changed_text(call, text).await?;
            modified = changed || modified;
        }
    }
    Ok(modified)
}

// unlike request masking, only a value the callback really changed counts
async fn mask_changed_text<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    text: &mut String,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let masked = call
        .guard
        .check_pii(
            text,
            call.output_parse_pii,
            call.settings,
            call.request_data,
        )
        .await?;
    if masked == *text {
        return Ok(false);
    }
    *text = masked;
    Ok(true)
}
