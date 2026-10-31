use betterlite_models::ocr_pricing::{
    OcrBatchUsage, OcrBilling, OcrCostError, OcrResponseKind, OcrUsage, layered_ocr_pricing,
    ocr_batch_cost, ocr_cost,
};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct UsageInput {
    credits: Option<rust_decimal::Decimal>,
    pages_processed: Option<i64>,
    pages_processed_annotation: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct BatchUsageInput {
    pages_processed: Option<i64>,
    pages_processed_annotation: Option<i64>,
}

// Mirrors the Python adapter's one-entry swap of litellm.model_cost for any model or provider asked.
#[derive(Clone, Deserialize)]
pub(crate) struct CatalogInput {
    #[serde(rename = "key")]
    _key: String,
    entry: PriceRecord,
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum Input {
    #[serde(rename = "layering")]
    Layering { sources: Vec<Option<PriceRecord>> },
    #[serde(rename = "sync")]
    Sync {
        model: String,
        custom_llm_provider: Option<String>,
        model_info: Option<PriceRecord>,
        usage: UsageInput,
        catalog: Option<CatalogInput>,
    },
    #[serde(rename = "batch")]
    Batch {
        model: String,
        custom_llm_provider: Option<String>,
        model_info: Option<PriceRecord>,
        usage: BatchUsageInput,
        catalog: Option<CatalogInput>,
    },
}

struct CaseLookup {
    catalog: Option<CatalogInput>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.catalog.as_ref().map(|catalog| catalog.entry.clone())
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        run_inner(input).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "OcrCostError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn decimal_json(value: Option<rust_decimal::Decimal>) -> serde_json::Value {
    match value {
        Some(decimal) => serde_json::Value::String(decimal.normalize().to_string()),
        None => serde_json::Value::Null,
    }
}

fn run_layering(sources: &[Option<PriceRecord>]) -> serde_json::Value {
    let refs: Vec<Option<&PriceRecord>> = sources.iter().map(Option::as_ref).collect();
    let pricing = layered_ocr_pricing(&refs);
    serde_json::json!({
        "ocr_cost_per_page": decimal_json(pricing.ocr_cost_per_page),
        "ocr_cost_per_credit": decimal_json(pricing.ocr_cost_per_credit),
        "annotation_cost_per_page": decimal_json(pricing.annotation_cost_per_page),
    })
}

fn run_sync(
    model: &str,
    custom_llm_provider: Option<&str>,
    model_info: Option<&PriceRecord>,
    usage: &UsageInput,
    catalog: Option<&CatalogInput>,
) -> Result<serde_json::Value, OcrCostError> {
    let billing = OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(OcrUsage {
            credits: usage.credits,
            pages_processed: usage.pages_processed,
            pages_processed_annotation: usage.pages_processed_annotation,
        }),
    };
    let lookup = CaseLookup {
        catalog: catalog.cloned(),
    };
    let cost = ocr_cost(model, custom_llm_provider, &billing, model_info, &lookup)?;
    Ok(serde_json::json!({ "cost": cost.normalize().to_string() }))
}

fn run_batch(
    model: &str,
    custom_llm_provider: Option<&str>,
    model_info: Option<&PriceRecord>,
    usage: &BatchUsageInput,
    catalog: Option<&CatalogInput>,
) -> Result<serde_json::Value, OcrCostError> {
    let batch_usage = OcrBatchUsage {
        pages_processed: usage.pages_processed,
        pages_processed_annotation: usage.pages_processed_annotation,
    };
    let lookup = CaseLookup {
        catalog: catalog.cloned(),
    };
    let cost = ocr_batch_cost(
        model,
        custom_llm_provider,
        &batch_usage,
        model_info,
        &lookup,
    )?;
    Ok(serde_json::json!({ "cost": cost.normalize().to_string() }))
}

fn run_inner(input: &Input) -> Result<serde_json::Value, OcrCostError> {
    match input {
        Input::Layering { sources } => Ok(run_layering(sources)),
        Input::Sync {
            model,
            custom_llm_provider,
            model_info,
            usage,
            catalog,
        } => run_sync(
            model,
            custom_llm_provider.as_deref(),
            model_info.as_ref(),
            usage,
            catalog.as_ref(),
        ),
        Input::Batch {
            model,
            custom_llm_provider,
            model_info,
            usage,
            catalog,
        } => run_batch(
            model,
            custom_llm_provider.as_deref(),
            model_info.as_ref(),
            usage,
            catalog.as_ref(),
        ),
    }
}

mod tests;
