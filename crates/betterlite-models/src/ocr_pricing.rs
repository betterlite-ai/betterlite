use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;
use rust_decimal::Decimal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrPricingField {
    OcrCostPerPage,
    OcrCostPerCredit,
    AnnotationCostPerPage,
}

impl OcrPricingField {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::OcrCostPerPage => "ocr_cost_per_page",
            Self::OcrCostPerCredit => "ocr_cost_per_credit",
            Self::AnnotationCostPerPage => "annotation_cost_per_page",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OcrPricing {
    pub ocr_cost_per_page: Option<Decimal>,
    pub ocr_cost_per_credit: Option<Decimal>,
    pub annotation_cost_per_page: Option<Decimal>,
}

// Only `Ocr` carries usable usage; `Other` always fails validate().
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrResponseKind {
    Ocr,
    Other,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OcrUsage {
    pub credits: Option<Decimal>,
    pub pages_processed: Option<i64>,
    pub pages_processed_annotation: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrBilling {
    pub kind: OcrResponseKind,
    pub usage: Option<OcrUsage>,
}

impl OcrBilling {
    // Rejects a response that was not OCR-shaped or carried no usage, before any lookup runs.
    pub fn validate(&self) -> Result<&OcrUsage, OcrCostError> {
        if self.kind != OcrResponseKind::Ocr {
            return Err(OcrCostError::WrongResponseKind);
        }
        self.usage.as_ref().ok_or(OcrCostError::MissingUsage)
    }
}

// Batch usage carries no credits field: ocr_batch_cost never bills credits.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OcrBatchUsage {
    pub pages_processed: Option<i64>,
    pub pages_processed_annotation: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum OcrCostError {
    #[error("OCR cost requires an OCR response")]
    WrongResponseKind,
    #[error("OCR response usage is missing")]
    MissingUsage,
    #[error(
        "OCR response pages_processed is missing for model={model} custom_llm_provider={provider}"
    )]
    MissingPages { model: String, provider: String },
    #[error("OCR cost overflowed Decimal arithmetic")]
    Overflow,
}

const BATCH_PAGE_RATE_KEYS: [&str; 2] = ["ocr_cost_per_page_batches", "ocr_cost_per_page"];
const BATCH_ANNOTATION_RATE_KEYS: [&str; 2] = [
    "annotation_cost_per_page_batches",
    "annotation_cost_per_page",
];

// A numeric scalar prices; a string, array, object or null never does, however it looks.
fn numeric_value(scalar: &Scalar) -> Option<Decimal> {
    match scalar {
        Scalar::Bool(flag) => Some(Decimal::from(u8::from(*flag))),
        Scalar::Number(number) => parse_decimal_lexeme(&number.to_string()).ok(),
        Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

#[must_use]
pub fn first_ocr_price(
    field: OcrPricingField,
    sources: &[Option<&PriceRecord>],
) -> Option<Decimal> {
    for source in sources.iter().flatten() {
        if let Some(value) = source.get(field.wire_name()).and_then(numeric_value) {
            return Some(value);
        }
    }
    None
}

#[must_use]
pub fn layered_ocr_pricing(sources: &[Option<&PriceRecord>]) -> OcrPricing {
    let ocr_cost_per_page = first_ocr_price(OcrPricingField::OcrCostPerPage, sources);
    let ocr_cost_per_credit = first_ocr_price(OcrPricingField::OcrCostPerCredit, sources);
    let annotation_cost_per_page = first_ocr_price(OcrPricingField::AnnotationCostPerPage, sources);
    OcrPricing {
        ocr_cost_per_page,
        ocr_cost_per_credit,
        annotation_cost_per_page,
    }
}

#[must_use]
pub fn first_price(source: Option<&PriceRecord>, keys: &[&str]) -> Option<Decimal> {
    let present_source = source?;
    keys.iter()
        .find_map(|key| present_source.get(*key).and_then(numeric_value))
}

// An absent provider is spelled "None" to match Python's str(None) in the same log line.
#[must_use]
pub fn single_log_line(value: Option<&str>) -> String {
    let Some(text) = value else {
        return "None".to_owned();
    };
    text.replace(['\r', '\n'], "")
}

fn checked_page_cost(rate: Option<Decimal>, pages: i64) -> Result<Decimal, OcrCostError> {
    let Some(cost) = rate
        .unwrap_or(Decimal::ZERO)
        .checked_mul(Decimal::from(pages))
    else {
        return Err(OcrCostError::Overflow);
    };
    Ok(cost)
}

// A page-priced model raises on a missing page count; any other model bills 0.0 instead.
fn missing_pages_outcome(
    model: &str,
    custom_llm_provider: Option<&str>,
    credits: Option<Decimal>,
    ocr_cost_per_credit: Option<Decimal>,
    ocr_cost_per_page: Option<Decimal>,
) -> Result<Decimal, OcrCostError> {
    if ocr_cost_per_credit.is_some() || ocr_cost_per_page.is_none() {
        tracing::warn!(
            model = model,
            custom_llm_provider = custom_llm_provider,
            credits = ?credits,
            "OCR cost: pages_processed is missing; returning 0.0 cost"
        );
        return Ok(Decimal::ZERO);
    }
    Err(OcrCostError::MissingPages {
        model: single_log_line(Some(model)),
        provider: single_log_line(custom_llm_provider),
    })
}

// Credits win over pages when both the usage and a credit rate are present.
pub fn ocr_cost(
    model: &str,
    custom_llm_provider: Option<&str>,
    billing: &OcrBilling,
    deployment_override: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, OcrCostError> {
    let usage = billing.validate()?;

    let effective = lookup.effective_model_info(model, custom_llm_provider);
    let pricing = layered_ocr_pricing(&[deployment_override, effective.as_ref()]);

    if let (Some(credits), Some(cost_per_credit)) = (usage.credits, pricing.ocr_cost_per_credit) {
        return cost_per_credit
            .checked_mul(credits)
            .ok_or(OcrCostError::Overflow);
    }

    let annotation_rate = pricing
        .annotation_cost_per_page
        .or(pricing.ocr_cost_per_page);
    let annotation_pages = usage.pages_processed_annotation.unwrap_or(0);
    let has_billable_annotation_pages = annotation_rate.is_some() && annotation_pages > 0;

    if usage.pages_processed.is_none() && !has_billable_annotation_pages {
        return missing_pages_outcome(
            model,
            custom_llm_provider,
            usage.credits,
            pricing.ocr_cost_per_credit,
            pricing.ocr_cost_per_page,
        );
    }

    if pricing.ocr_cost_per_page.is_none() && !has_billable_annotation_pages {
        tracing::warn!(
            model = model,
            custom_llm_provider = custom_llm_provider,
            pages_processed = ?usage.pages_processed,
            "OCR cost: no ocr_cost_per_page is configured; returning 0.0 cost"
        );
        return Ok(Decimal::ZERO);
    }

    let page_cost = checked_page_cost(
        pricing.ocr_cost_per_page,
        usage.pages_processed.unwrap_or(0),
    )?;
    let annotation_cost = checked_page_cost(annotation_rate, annotation_pages)?;
    page_cost
        .checked_add(annotation_cost)
        .ok_or(OcrCostError::Overflow)
}

// Page and annotation families resolve independently; a deployment rate beats the public cost map.
pub fn ocr_batch_cost(
    model: &str,
    custom_llm_provider: Option<&str>,
    usage: &OcrBatchUsage,
    deployment_info: Option<&PriceRecord>,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, OcrCostError> {
    let pages_processed = usage.pages_processed.unwrap_or(0);
    let annotation_pages = usage.pages_processed_annotation.unwrap_or(0);

    let deployment_page_rate = first_price(deployment_info, &BATCH_PAGE_RATE_KEYS);
    let deployment_annotation_rate = first_price(deployment_info, &BATCH_ANNOTATION_RATE_KEYS);
    let needs_published_pricing = (pages_processed > 0 && deployment_page_rate.is_none())
        || (annotation_pages > 0 && deployment_annotation_rate.is_none());

    let published = if needs_published_pricing {
        lookup.effective_model_info(model, custom_llm_provider)
    } else {
        None
    };
    if needs_published_pricing && published.is_none() {
        tracing::warn!(
            model = single_log_line(Some(model)),
            custom_llm_provider = single_log_line(custom_llm_provider),
            "OCR batch cost: no pricing entry; billing only the families the deployment prices"
        );
    }

    let page_rate =
        deployment_page_rate.or_else(|| first_price(published.as_ref(), &BATCH_PAGE_RATE_KEYS));
    let annotation_rate = deployment_annotation_rate
        .or_else(|| first_price(published.as_ref(), &BATCH_ANNOTATION_RATE_KEYS));
    if page_rate.is_none() && pages_processed > 0 {
        tracing::warn!(
            model = single_log_line(Some(model)),
            custom_llm_provider = single_log_line(custom_llm_provider),
            pages_processed = pages_processed,
            "OCR batch cost: no ocr_cost_per_page is configured; returning 0.0 cost for those pages"
        );
    }
    let effective_annotation_rate = annotation_rate.or(page_rate);

    let page_cost = checked_page_cost(page_rate, pages_processed)?;
    let annotation_cost = checked_page_cost(effective_annotation_rate, annotation_pages)?;
    page_cost
        .checked_add(annotation_cost)
        .ok_or(OcrCostError::Overflow)
}

mod tests;
