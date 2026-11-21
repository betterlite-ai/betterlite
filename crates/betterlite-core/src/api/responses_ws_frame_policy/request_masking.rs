use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::PresidioGuardrailCallback;
use indexmap::IndexMap;

use super::{MASKABLE_TEXT_BLOCK_TYPES, PiiCall};

// a client may put its text at the top level or under response, so both shapes are masked
pub(crate) async fn mask_create_frame<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    frame: &mut IndexMap<String, Json>,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let top = mask_create_container(call, frame).await?;
    let Some(Json::Object(nested)) = frame.get_mut("response") else {
        return Ok(top);
    };
    let inner = mask_create_container(call, nested).await?;
    Ok(top || inner)
}

async fn mask_create_container<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    container: &mut IndexMap<String, Json>,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let mut masked = false;
    if let Some(input) = container.get_mut("input") {
        masked = mask_create_field(call, input).await?;
    }
    if let Some(instructions @ Json::Text(_)) = container.get_mut("instructions") {
        masked = mask_create_field(call, instructions).await? || masked;
    }
    Ok(masked)
}

async fn mask_create_field<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    value: &mut Json,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let items = match value {
        Json::Text(text) => {
            let replaced = call
                .guard
                .check_pii(
                    text,
                    call.output_parse_pii,
                    call.settings,
                    call.request_data,
                )
                .await?;
            *text = replaced;
            return Ok(true);
        }
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => return Ok(false),
    };
    let mut masked = false;
    for item in items {
        if let Json::Object(fields) = item {
            let changed = mask_create_item(call, fields).await?;
            masked = changed || masked;
        }
    }
    Ok(masked)
}

async fn mask_create_item<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    item: &mut IndexMap<String, Json>,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let mut masked = false;
    for field in ["content", "output"] {
        match item.get_mut(field) {
            Some(Json::Text(text)) => {
                let replaced = call
                    .guard
                    .check_pii(
                        text,
                        call.output_parse_pii,
                        call.settings,
                        call.request_data,
                    )
                    .await?;
                *text = replaced;
                masked = true;
            }
            Some(Json::List(blocks)) => {
                let changed = mask_create_blocks(call, blocks).await?;
                masked = changed || masked;
            }
            None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {}
        }
    }
    Ok(masked)
}

// an image or unknown block type keeps its text, since only these three carry text a model reads
async fn mask_create_blocks<Guard>(
    call: &PiiCall<'_, Guard, Guard::Settings>,
    blocks: &mut [Json],
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let mut masked = false;
    for block in blocks {
        let Json::Object(fields) = block else {
            continue;
        };
        let maskable = matches!(
            fields.get("type"),
            Some(Json::Text(kind)) if MASKABLE_TEXT_BLOCK_TYPES.contains(&kind.as_str())
        );
        if maskable && let Some(Json::Text(text)) = fields.get_mut("text") {
            let replaced = call
                .guard
                .check_pii(
                    text,
                    call.output_parse_pii,
                    call.settings,
                    call.request_data,
                )
                .await?;
            *text = replaced;
            masked = true;
        }
    }
    Ok(masked)
}
