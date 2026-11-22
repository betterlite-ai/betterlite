use std::borrow::Cow;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::PresidioGuardrailCallback;
use indexmap::IndexMap;

use super::{
    DELTA_EVENT_TYPES, FramePolicy, FramePolicyError, PiiCall, RequestData, UnmaskPiiText,
    decode_object,
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
        let unmasking = Unmasking {
            unmask,
            pii_tokens: &pii_tokens,
        };
        return if unmasking.completed(&mut event) {
            Ok(serde_json::to_string(&Json::Object(event)).map(Cow::Owned)?)
        } else {
            Ok(Cow::Borrowed(message))
        };
    }
    if streamed_delta && let Some(Json::Text(delta)) = event.get_mut("delta") {
        let unmasked = unmask(delta, &pii_tokens);
        if unmasked != *delta {
            *delta = unmasked;
            return Ok(serde_json::to_string(&Json::Object(event)).map(Cow::Owned)?);
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

struct Unmasking<'policy> {
    unmask: &'policy UnmaskPiiText,
    pii_tokens: &'policy IndexMap<String, String>,
}

impl Unmasking<'_> {
    fn completed(&self, event: &mut IndexMap<String, Json>) -> bool {
        let Some(Json::Object(response)) = event.get_mut("response") else {
            return false;
        };
        let Some(Json::List(items)) = response.get_mut("output") else {
            return false;
        };
        let mut modified = false;
        for item in items {
            let Json::Object(fields) = item else {
                continue;
            };
            let Some(Json::List(blocks)) = fields.get_mut("content") else {
                continue;
            };
            modified |= self.blocks(blocks);
        }
        modified
    }

    fn blocks(&self, blocks: &mut [Json]) -> bool {
        let mut modified = false;
        for text in blocks.iter_mut().filter_map(text_field) {
            let unmasked = (self.unmask)(text, self.pii_tokens);
            if unmasked != *text {
                *text = unmasked;
                modified = true;
            }
        }
        modified
    }
}

fn text_field(block: &mut Json) -> Option<&mut String> {
    let Json::Object(fields) = block else {
        return None;
    };
    text_at(fields, "text")
}

fn text_at<'map>(fields: &'map mut IndexMap<String, Json>, key: &str) -> Option<&'map mut String> {
    let Json::Text(text) = fields.get_mut(key)? else {
        return None;
    };
    Some(text)
}

fn block_list<'map>(
    fields: &'map mut IndexMap<String, Json>,
    key: &str,
) -> Option<&'map mut [Json]> {
    let Json::List(blocks) = fields.get_mut(key)? else {
        return None;
    };
    Some(blocks)
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
        let masking = OutputMasking { call: &call };
        if let Some(Json::Object(response)) = event.get_mut("response")
            && let Some(Json::List(items)) = response.get_mut("output")
        {
            let changed = masking
                .items(items)
                .await
                .map_err(FramePolicyError::Guardrail)?;
            modified = changed || modified;
        }
    }
    if modified {
        Ok(serde_json::to_string(&Json::Object(event)).map(Cow::Owned)?)
    } else {
        Ok(Cow::Borrowed(message))
    }
}

struct OutputMasking<'call, Guard>
where
    Guard: PresidioGuardrailCallback,
{
    call: &'call PiiCall<'call, Guard>,
}

impl<Guard> OutputMasking<'_, Guard>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    async fn items(&self, items: &mut [Json]) -> Result<bool, Guard::Error> {
        let mut modified = false;
        for item in items {
            let Json::Object(fields) = item else {
                continue;
            };
            modified |= self.item(fields).await?;
        }
        Ok(modified)
    }

    async fn item(&self, fields: &mut IndexMap<String, Json>) -> Result<bool, Guard::Error> {
        let mut modified = false;
        if let Some(arguments) = text_at(fields, "arguments") {
            modified |= self.changed(arguments).await?;
        }
        for blocks_field in ["summary", "content"] {
            let Some(blocks) = block_list(fields, blocks_field) else {
                continue;
            };
            modified |= self.blocks(blocks).await?;
        }
        Ok(modified)
    }

    async fn blocks(&self, blocks: &mut [Json]) -> Result<bool, Guard::Error> {
        let mut modified = false;
        for text in blocks.iter_mut().filter_map(text_field) {
            modified |= self.changed(text).await?;
        }
        Ok(modified)
    }

    // unlike request masking, only a value the callback really changed counts
    async fn changed(&self, text: &mut String) -> Result<bool, Guard::Error> {
        let masked = self.call.check_text(text).await?;
        if masked == *text {
            return Ok(false);
        }
        *text = masked;
        Ok(true)
    }
}
