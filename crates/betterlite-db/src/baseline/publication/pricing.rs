use betterlite_models::baseline_history::estimate::{BaselineEstimate, Provenance};
use betterlite_models::baseline_history::usage::{
    CacheCreationTokenDetails as BaselineCreationDetails,
    PromptTokensDetails as BaselinePromptDetails, Usage as BaselineUsage,
};
use betterlite_models::cost_usage::CompletionDetails;
use betterlite_models::cost_usage::modality_projection::CacheCreationTokenDetails;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use betterlite_models::savings_pricing::compare::{
    ComparisonInput, ComparisonProvenance, price_baseline_comparison,
};
use betterlite_models::savings_pricing::context::SavingsPricingContext;
use betterlite_models::savings_pricing::usage::RequestUsage;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use super::model::{BaselinePublication, PublicationStatus};
use crate::baseline::records::BaselineAccountingRecord;

// Everything the source's comparison pricing reads: the injected price side, the tz database and the pricing moment.
pub struct PublicationPricing<'a, C> {
    pub context: &'a C,
    pub database: TimeZoneDatabase,
    pub moment: DateTime<Utc>,
}

impl<'a, C> PublicationPricing<'a, C> {
    // The process-wide zone database is what the source's price call reads when no caller supplies one.
    #[must_use]
    pub fn new(context: &'a C, moment: DateTime<Utc>) -> Self {
        Self {
            context,
            database: TimeZoneDatabase::from_env(),
            moment,
        }
    }
}

// baseline_publication: price the estimate against the recorded snapshot, keeping its token evidence either way.
pub fn baseline_publication<C>(
    pricing: &PublicationPricing<'_, C>,
    record: &BaselineAccountingRecord,
    estimate: &BaselineEstimate,
    first_at: f64,
) -> BaselinePublication
where
    C: SavingsPricingContext,
{
    let usage = estimate.usage.as_ref();
    let request = usage.map(request_usage);
    let provenance = estimate.provenance.map(comparison_provenance);
    let costs = price_baseline_comparison(
        pricing.context,
        &ComparisonInput {
            snapshot: &record.pricing,
            baseline_usage: request.as_ref(),
            provenance,
            moment: pricing.moment,
            database: &pricing.database,
        },
    );
    let details = usage.and_then(|value| value.prompt_tokens_details.as_ref());
    let writes = details.and_then(|value| value.cache_creation_token_details.as_ref());
    let estimated = costs.is_some();
    BaselinePublication {
        comparison_id: record.scope.clone(),
        comparison_started_at: first_at,
        status: if estimated {
            PublicationStatus::Estimated
        } else {
            PublicationStatus::Unknown
        },
        reason: if estimated || usage.is_none() {
            estimate.reason.clone()
        } else {
            "pricing_unavailable".to_owned()
        },
        provenance: if estimated { provenance } else { None },
        actual_spend: costs.map(|value| value.actual),
        baseline_spend: costs.map(|value| value.baseline),
        input_tokens: details.and_then(|value| value.text_tokens),
        cache_read_input_tokens: details.and_then(|value| value.cached_tokens),
        cache_creation_5m_input_tokens: writes.and_then(|value| value.ephemeral_5m_input_tokens),
        cache_creation_1h_input_tokens: writes.and_then(|value| value.ephemeral_1h_input_tokens),
    }
}

const fn comparison_provenance(value: Provenance) -> ComparisonProvenance {
    match value {
        Provenance::ObservedIdentical => ComparisonProvenance::ObservedIdentical,
        Provenance::Modeled => ComparisonProvenance::Modeled,
    }
}

fn request_usage(usage: &BaselineUsage) -> RequestUsage {
    let details = usage.prompt_tokens_details.as_ref();
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
            cache_creation_input_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
            prompt_tokens_details: details.map(prompt_details),
            completion_tokens_details: completion_details(
                usage.extra.get("completion_tokens_details"),
            ),
        },
        cache_creation_token_details: details
            .and_then(|value| value.cache_creation_token_details.as_ref())
            .map(ttl_details),
    }
}

fn prompt_details(value: &BaselinePromptDetails) -> PromptTokensDetails {
    PromptTokensDetails {
        cached_tokens: value.cached_tokens.unwrap_or_default(),
        cache_creation_tokens: value.cache_creation_tokens.unwrap_or_default(),
        cache_write_tokens: value.cache_write_tokens.unwrap_or_default(),
        text_tokens: value.text_tokens.unwrap_or_default(),
        audio_tokens: extra_count(&value.extra, "audio_tokens"),
        image_tokens: extra_count(&value.extra, "image_tokens"),
        video_tokens: extra_count(&value.extra, "video_tokens"),
        cache_creation_token_details: None,
    }
}

fn ttl_details(value: &BaselineCreationDetails) -> CacheCreationTokenDetails {
    CacheCreationTokenDetails {
        ephemeral_5m_input_tokens: value.ephemeral_5m_input_tokens,
        ephemeral_1h_input_tokens: value.ephemeral_1h_input_tokens,
    }
}

fn completion_details(value: Option<&Json>) -> Option<CompletionDetails> {
    let Some(Json::Object(fields)) = value else {
        return None;
    };
    Some(CompletionDetails {
        text_tokens: optional_count(fields.get("text_tokens")),
        audio_tokens: optional_count(fields.get("audio_tokens")),
        image_tokens: optional_count(fields.get("image_tokens")),
        video_tokens: optional_count(fields.get("video_tokens")),
        reasoning_tokens: optional_count(fields.get("reasoning_tokens")),
    })
}

fn extra_count(extra: &Extra, key: &str) -> i64 {
    optional_count(extra.get(key)).unwrap_or_default()
}

fn optional_count(value: Option<&Json>) -> Option<i64> {
    let Some(Json::Number(number)) = value else {
        return None;
    };
    if number.to_string().contains(['.', 'e', 'E']) {
        return None;
    }
    number.as_i64()
}
