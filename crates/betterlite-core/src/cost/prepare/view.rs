use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::ResponseView;
use super::error::PrepareError;
use crate::cost::context::CostResponse;
use crate::cost::usage::{BillingResponse, BillingUsage, ResponseKind};

enum HiddenValue<'hidden> {
    Absent,
    Null,
    Text(&'hidden str),
}

fn hidden_value<'hidden>(
    hidden: &'hidden IndexMap<String, Json>,
    field: &'static str,
) -> Result<HiddenValue<'hidden>, PrepareError> {
    match hidden.get(field) {
        None => Ok(HiddenValue::Absent),
        Some(Json::Null) => Ok(HiddenValue::Null),
        Some(Json::Text(text)) => Ok(HiddenValue::Text(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PrepareError::Hidden { field })
        }
    }
}

// Rebuilding per variant keeps the references Copy without a Copy derive on BillingUsage
const fn copied_usage<'usage>(usage: &'usage BillingUsage<'_>) -> BillingUsage<'usage> {
    match usage {
        BillingUsage::Absent => BillingUsage::Absent,
        BillingUsage::Null => BillingUsage::Null,
        BillingUsage::Chat(found) => BillingUsage::Chat(found),
        BillingUsage::Response(found) => BillingUsage::Response(found),
        BillingUsage::Transcription(found) => BillingUsage::Transcription(found),
        BillingUsage::DecisionsIr(found) => BillingUsage::DecisionsIr(found),
        BillingUsage::DecisionsOpenAi(found) => BillingUsage::DecisionsOpenAi(found),
        BillingUsage::Interactions(found) => BillingUsage::Interactions(found),
        BillingUsage::Mapping(found) => BillingUsage::Mapping(found),
        BillingUsage::TypedRecord(found) => BillingUsage::TypedRecord(found),
    }
}

impl<'response> ResponseView<'response> {
    pub(super) const fn kind(&self) -> ResponseKind {
        match self {
            Self::Typed(typed) => typed.kind,
            Self::Realtime { .. } | Self::Dictionary(_) | Self::Other => ResponseKind::Other,
        }
    }

    pub(super) const fn usage(&self) -> Option<BillingUsage<'_>> {
        let found = match self {
            Self::Realtime { usage, .. } => BillingUsage::Chat(usage),
            Self::Typed(typed) => copied_usage(&typed.usage),
            Self::Dictionary(dictionary) => copied_usage(&dictionary.usage),
            Self::Other => return None,
        };
        Some(found)
    }

    pub(super) fn billing(&self) -> BillingResponse<'_> {
        BillingResponse {
            kind: self.kind(),
            usage: self.usage().unwrap_or(BillingUsage::Absent),
            service_tier: self.service_tier(),
            hidden_params: self.hidden_params(),
        }
    }

    pub(super) const fn model(&self) -> Option<&str> {
        match self {
            Self::Typed(typed) => typed.model,
            Self::Dictionary(dictionary) => dictionary.model,
            Self::Realtime { .. } | Self::Other => None,
        }
    }

    pub(super) fn service_tier(&self) -> Option<&str> {
        match self {
            Self::Realtime { record, .. } => record.service_tier.as_deref(),
            Self::Typed(typed) => typed.service_tier,
            Self::Dictionary(dictionary) => dictionary.service_tier,
            Self::Other => None,
        }
    }

    pub(super) const fn hidden_params(&self) -> Option<&IndexMap<String, Json>> {
        match self {
            Self::Typed(typed) => typed.hidden_params,
            Self::Realtime { .. } | Self::Dictionary(_) | Self::Other => None,
        }
    }

    pub(super) const fn cost_response(&self) -> Option<CostResponse<'response>> {
        if let Self::Typed(typed) = self {
            return Some(CostResponse::Object {
                model: typed.model,
                hidden_params: typed.hidden_params,
                response_ms: None,
            });
        }
        if let Self::Dictionary(dictionary) = self {
            return Some(CostResponse::Dictionary {
                model: dictionary.model,
            });
        }
        if let Self::Realtime { .. } = self {
            return Some(CostResponse::Object {
                model: None,
                hidden_params: None,
                response_ms: None,
            });
        }
        None
    }

    pub(super) fn rebind(
        &self,
        provider: &mut Option<String>,
        region: &mut Option<String>,
    ) -> Result<(), PrepareError> {
        match self {
            Self::Realtime { .. } => {
                let current = provider.take();
                *provider = current.filter(|name| !name.is_empty());
                Ok(())
            }
            Self::Typed(typed) => {
                let Some(hidden) = typed.hidden_params else {
                    return Ok(());
                };
                let current_provider = provider.take();
                *provider = match hidden_value(hidden, "custom_llm_provider")? {
                    HiddenValue::Absent => current_provider.filter(|name| !name.is_empty()),
                    HiddenValue::Null => None,
                    HiddenValue::Text(found) => Some(found.to_owned()),
                };
                let current_region = region.take();
                *region = match hidden_value(hidden, "region_name")? {
                    HiddenValue::Absent => current_region,
                    HiddenValue::Null => None,
                    HiddenValue::Text(found) => Some(found.to_owned()),
                };
                Ok(())
            }
            Self::Dictionary(_) | Self::Other => Ok(()),
        }
    }
}
