use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::websocket::PresidioGuardrailCallback;
use indexmap::IndexMap;

use super::{MASKABLE_TEXT_BLOCK_TYPES, PiiCall};

// a client may put its text at the top level or under response, so both shapes are masked
pub(crate) async fn mask_create_frame<Guard>(
    call: &PiiCall<'_, Guard>,
    frame: &mut IndexMap<String, Json>,
) -> Result<bool, Guard::Error>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    let masking = Masking { call };
    let mut masked = masking.container(frame).await?;
    if let Some(Json::Object(nested)) = frame.get_mut("response") {
        masked |= masking.container(nested).await?;
    }
    Ok(masked)
}

struct Masking<'call, Guard>
where
    Guard: PresidioGuardrailCallback,
{
    call: &'call PiiCall<'call, Guard>,
}

impl<Guard> Masking<'_, Guard>
where
    Guard: PresidioGuardrailCallback + Sync,
    Guard::Settings: Sync,
{
    async fn container(
        &self,
        container: &mut IndexMap<String, Json>,
    ) -> Result<bool, Guard::Error> {
        let mut masked = if let Some(input) = container.get_mut("input") {
            self.input(input).await?
        } else {
            false
        };
        if let Some(Json::Text(text)) = container.get_mut("instructions") {
            masked |= self.text(text).await?;
        }
        Ok(masked)
    }

    // input is a string or a list of items whose content and output fields carry the text
    async fn input(&self, value: &mut Json) -> Result<bool, Guard::Error> {
        let items = match value {
            Json::Text(text) => return self.text(text).await,
            Json::List(items) => items,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => return Ok(false),
        };
        let mut masked = false;
        for item in items {
            let Json::Object(fields) = item else {
                continue;
            };
            if let Some(content) = fields.get_mut("content") {
                masked |= self.field(content).await?;
            }
            if let Some(output) = fields.get_mut("output") {
                masked |= self.field(output).await?;
            }
        }
        Ok(masked)
    }

    async fn field(&self, value: &mut Json) -> Result<bool, Guard::Error> {
        if let Json::List(blocks) = value {
            return self.blocks(blocks).await;
        }
        let Json::Text(text) = value else {
            return Ok(false);
        };
        self.text(text).await
    }

    async fn text(&self, text: &mut String) -> Result<bool, Guard::Error> {
        let replaced = self.call.check_text(text).await?;
        *text = replaced;
        Ok(true)
    }

    async fn blocks(&self, blocks: &mut [Json]) -> Result<bool, Guard::Error> {
        let mut masked = false;
        for text in blocks.iter_mut().filter_map(maskable_text) {
            masked |= self.text(text).await?;
        }
        Ok(masked)
    }
}

// an image or unknown block type keeps its text, since only these three carry text a model reads
fn maskable_text(value: &mut Json) -> Option<&mut String> {
    let Json::Object(fields) = value else {
        return None;
    };
    let carries_text = matches!(
        fields.get("type"),
        Some(Json::Text(kind)) if MASKABLE_TEXT_BLOCK_TYPES.contains(&kind.as_str())
    );
    if !carries_text {
        return None;
    }
    let Json::Text(text) = fields.get_mut("text")? else {
        return None;
    };
    Some(text)
}
