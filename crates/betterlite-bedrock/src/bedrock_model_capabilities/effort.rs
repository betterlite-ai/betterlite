use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;

use crate::model_routes::ParamValue;

use super::{CapabilityError, MODEL_KEY_FIELD, ModelInfoLookup};

const EFFORT_CEILING_FIELD: &str = "bedrock_output_config_effort_ceiling";
const EFFORT_FIELD: &str = "effort";

// LiteLLM's literal rank table, where xhigh sits above max
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Effort {
    Low,
    Medium,
    High,
    Max,
    Xhigh,
}

impl Effort {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            "xhigh" => Some(Self::Xhigh),
            _ => None,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::Xhigh => "xhigh",
        }
    }
}

pub async fn normalize_bedrock_opus_output_config_effort<Lookup: ModelInfoLookup>(
    model: &str,
    output_config: &mut ParamValue,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> Result<(), CapabilityError> {
    let ParamValue::Mapping(fields) = output_config else {
        return Ok(());
    };
    let Some(declared) = fields.get(EFFORT_FIELD) else {
        return Ok(());
    };
    let Some(effort) = effort_rank(declared)? else {
        return Ok(());
    };
    let Some(ceiling) = effort_ceiling(model, lookup, packaged).await else {
        return Ok(());
    };
    if effort > ceiling {
        fields.insert(
            EFFORT_FIELD.to_owned(),
            ParamValue::Text(ceiling.text().to_owned()),
        );
    }
    Ok(())
}

fn effort_rank(effort: &ParamValue) -> Result<Option<Effort>, CapabilityError> {
    match effort {
        ParamValue::Text(text) => Ok(Effort::parse(text)),
        ParamValue::List(_) | ParamValue::Mapping(_) => Err(CapabilityError::UnhashableEffort),
        ParamValue::Null | ParamValue::Bool(_) | ParamValue::Number(_) => Ok(None),
    }
}

async fn effort_ceiling<Lookup: ModelInfoLookup>(
    model: &str,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> Option<Effort> {
    let info = lookup.bedrock_model_info(model).await.ok()?;
    if let Some(Json::Text(text)) = info.fields.get(EFFORT_CEILING_FIELD)
        && let Some(effort) = Effort::parse(text)
    {
        return Some(effort);
    }
    let Some(Json::Text(key)) = info.fields.get(MODEL_KEY_FIELD) else {
        return None;
    };
    let ceiling = packaged
        .entries
        .get(key)?
        .fields
        .metadata
        .bedrock_output_ceiling?;
    packaged_effort(ceiling)
}

fn packaged_effort(ceiling: ReasoningEffort) -> Option<Effort> {
    match ceiling {
        ReasoningEffort::Low => Some(Effort::Low),
        ReasoningEffort::Medium => Some(Effort::Medium),
        ReasoningEffort::High => Some(Effort::High),
        ReasoningEffort::Max => Some(Effort::Max),
        ReasoningEffort::Xhigh => Some(Effort::Xhigh),
        ReasoningEffort::None | ReasoningEffort::Minimal => None,
    }
}
