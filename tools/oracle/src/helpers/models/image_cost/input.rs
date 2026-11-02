use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_models::image_cost::usage::{DetailMap, ImageUsageDetails};
use betterlite_models::image_cost::{ImageBilling, ImageUsageView};
use betterlite_models::price_lookup::PriceRecord;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

use crate::helpers::HelperError;

pub(crate) const fn default_moment() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

const fn default_image_count() -> i64 {
    1
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub(crate) struct PromptDetailsInput {
    #[serde(default)]
    pub(crate) text_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) image_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) video_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) web_search_requests: Option<i64>,
}

impl From<PromptDetailsInput> for PromptDetails {
    fn from(fields: PromptDetailsInput) -> Self {
        Self {
            text_tokens: fields.text_tokens,
            audio_tokens: fields.audio_tokens,
            image_tokens: fields.image_tokens,
            video_tokens: fields.video_tokens,
            web_search_requests: fields.web_search_requests,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub(crate) struct CompletionDetailsInput {
    #[serde(default)]
    pub(crate) text_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) audio_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) image_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) video_tokens: Option<i64>,
    #[serde(default, rename = "reasoning_tokens")]
    pub(crate) reasoning: Option<i64>,
}

impl From<CompletionDetailsInput> for CompletionDetails {
    fn from(fields: CompletionDetailsInput) -> Self {
        Self {
            text_tokens: fields.text_tokens,
            audio_tokens: fields.audio_tokens,
            image_tokens: fields.image_tokens,
            video_tokens: fields.video_tokens,
            reasoning_tokens: fields.reasoning,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ChatUsageInput {
    #[serde(default)]
    pub(crate) prompt_tokens: i64,
    #[serde(default)]
    pub(crate) completion_tokens: i64,
    #[serde(default)]
    pub(crate) total_tokens: i64,
    #[serde(default)]
    pub(crate) prompt_details: Option<PromptDetailsInput>,
    #[serde(default)]
    pub(crate) completion_details: Option<CompletionDetailsInput>,
    #[serde(default)]
    pub(crate) cache_read_tokens: Option<i64>,
}

impl From<ChatUsageInput> for CostUsage {
    fn from(fields: ChatUsageInput) -> Self {
        Self {
            prompt_tokens: fields.prompt_tokens,
            completion_tokens: fields.completion_tokens,
            total_tokens: fields.total_tokens,
            prompt_details: fields.prompt_details.map(PromptDetails::from),
            completion_details: fields.completion_details.map(CompletionDetails::from),
            cache_read_tokens: fields.cache_read_tokens,
            numeric_extras: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct ImageUsageDetailsInput {
    #[serde(default)]
    pub(crate) input_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) output_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) total_tokens: Option<i64>,
    #[serde(default)]
    pub(crate) input_tokens_details: Option<Value>,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<Value>,
    #[serde(default)]
    pub(crate) output_tokens_details: Option<Value>,
}

// A present detail object, even empty, carries real record semantics distinct from an absent field.
fn detail_map(field: Option<&Value>) -> Result<Option<DetailMap>, Box<HelperError>> {
    let Some(present) = field else {
        return Ok(None);
    };
    let Value::Object(entries) = present else {
        return Err(input_error("a token detail field must be a JSON object"));
    };
    let mut map = DetailMap::new();
    for (key, entry) in entries {
        let text = serde_json::to_string(entry).map_err(|error| input_error(&error.to_string()))?;
        let raw = RawValue::from_string(text).map_err(|error| input_error(&error.to_string()))?;
        map.insert(key.clone().into_boxed_str(), raw);
    }
    Ok(Some(map))
}

fn input_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

impl ImageUsageDetailsInput {
    pub(crate) fn to_details(&self) -> Result<ImageUsageDetails, Box<HelperError>> {
        Ok(ImageUsageDetails {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.total_tokens,
            input_tokens_details: detail_map(self.input_tokens_details.as_ref())?,
            completion_tokens_details: detail_map(self.completion_tokens_details.as_ref())?,
            output_tokens_details: detail_map(self.output_tokens_details.as_ref())?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum UsageViewInput {
    #[serde(rename = "chat")]
    Chat(ChatUsageInput),
    #[serde(rename = "image")]
    Image(ImageUsageDetailsInput),
}

impl UsageViewInput {
    pub(crate) fn into_view(self) -> Result<ImageUsageView, Box<HelperError>> {
        match self {
            Self::Chat(fields) => Ok(ImageUsageView::Chat(CostUsage::from(fields))),
            Self::Image(fields) => Ok(ImageUsageView::Image(fields.to_details()?)),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct BillingInput {
    #[serde(default)]
    pub(crate) image_count: i64,
    #[serde(default)]
    pub(crate) size: Option<String>,
    #[serde(default)]
    pub(crate) quality: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<UsageViewInput>,
}

impl BillingInput {
    pub(crate) fn into_billing(self) -> Result<ImageBilling, Box<HelperError>> {
        Ok(ImageBilling {
            image_count: self.image_count,
            size: self.size,
            quality: self.quality,
            usage: self.usage.map(UsageViewInput::into_view).transpose()?,
        })
    }
}

pub(crate) fn raw_value(field: Option<&Value>) -> Result<Option<Box<RawValue>>, Box<HelperError>> {
    let Some(present) = field else {
        return Ok(None);
    };
    let text = serde_json::to_string(present).map_err(|error| input_error(&error.to_string()))?;
    RawValue::from_string(text)
        .map(Some)
        .map_err(|error| input_error(&error.to_string()))
}

#[derive(Debug, Deserialize)]
pub(crate) struct DefaultInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) quality: Option<String>,
    #[serde(default)]
    pub(crate) n_images: Option<i64>,
    #[serde(default)]
    pub(crate) size: Option<String>,
    #[serde(default)]
    pub(crate) model_info: Option<PriceRecord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenaiInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) model_info: Option<PriceRecord>,
    #[serde(default = "default_image_count")]
    pub(crate) image_count: i64,
    #[serde(default)]
    pub(crate) usage: Option<UsageViewInput>,
    #[serde(default = "default_moment")]
    pub(crate) moment: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AzureInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) model_info: Option<PriceRecord>,
    #[serde(default)]
    pub(crate) size: Option<String>,
    #[serde(default)]
    pub(crate) n_images: Option<i64>,
    #[serde(default)]
    pub(crate) width: Option<Value>,
    #[serde(default)]
    pub(crate) height: Option<Value>,
    pub(crate) billing: BillingInput,
    #[serde(default = "default_moment")]
    pub(crate) moment: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UsageOpInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
    #[serde(default)]
    pub(crate) model_info: Option<PriceRecord>,
    pub(crate) usage: ImageUsageDetailsInput,
    #[serde(default = "default_moment")]
    pub(crate) moment: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "default")]
    Default(DefaultInput),
    #[serde(rename = "openai")]
    Openai(OpenaiInput),
    #[serde(rename = "azure")]
    Azure(Box<AzureInput>),
    #[serde(rename = "usage")]
    Usage(UsageOpInput),
}
