use rust_decimal::prelude::ToPrimitive as _;

use super::dynamic::MappingOrScalar;
use super::error::SavingsEvidenceError;
use super::metadata::{CompressionSavings, GuardrailEntry, GuardrailInformation, Metadata};
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

const HEADROOM_GUARDRAIL_PROVIDER: &str = "headroom";

pub fn compression_saved_tokens(metadata: &Metadata) -> Result<i64, SavingsEvidenceError> {
    let native = tokens_saved_from_stats(metadata.compression_savings.as_ref())?;
    let headroom = headroom_saved_tokens(metadata.guardrail_information.as_ref())?;
    native
        .checked_add(headroom)
        .ok_or(SavingsEvidenceError::SavedTokensOverflow)
}

// A bool is excluded before the number match; a negative value or a numeric string both read as zero.
fn saved_tokens_or_zero(value: Option<&Scalar>) -> Result<i64, SavingsEvidenceError> {
    let Some(Scalar::Number(number)) = value else {
        return Ok(0);
    };
    let decimal = parse_decimal_lexeme(&number.to_string())
        .map_err(|_source| SavingsEvidenceError::SavedTokensOverflow)?;
    if decimal.is_sign_negative() {
        return Ok(0);
    }
    decimal
        .trunc()
        .to_i64()
        .ok_or(SavingsEvidenceError::SavedTokensOverflow)
}

fn tokens_saved_from_stats(
    stats: Option<&MappingOrScalar<CompressionSavings>>,
) -> Result<i64, SavingsEvidenceError> {
    let Some(mapping) = stats.and_then(MappingOrScalar::as_mapping) else {
        return Ok(0);
    };
    saved_tokens_or_zero(mapping.tokens_saved.as_ref())
}

fn headroom_entry_saved_tokens(entry: &GuardrailEntry) -> Result<i64, SavingsEvidenceError> {
    let provider_matches = matches!(&entry.guardrail_provider, Some(Scalar::Text(provider)) if provider == HEADROOM_GUARDRAIL_PROVIDER);
    if !provider_matches {
        return Ok(0);
    }
    tokens_saved_from_stats(entry.guardrail_response.as_ref())
}

// A bare mapping is one entry; a list is many entries; anything else carries none.
fn headroom_saved_tokens(
    guardrail_information: Option<&GuardrailInformation>,
) -> Result<i64, SavingsEvidenceError> {
    let entries: Vec<&GuardrailEntry> = match guardrail_information {
        Some(GuardrailInformation::One(entry)) => vec![entry],
        Some(GuardrailInformation::Many(entries)) => entries
            .iter()
            .filter_map(MappingOrScalar::as_mapping)
            .collect(),
        Some(GuardrailInformation::Other(_)) | None => return Ok(0),
    };
    entries.into_iter().try_fold(0_i64, |total, entry| {
        let contribution = headroom_entry_saved_tokens(entry)?;
        total
            .checked_add(contribution)
            .ok_or(SavingsEvidenceError::SavedTokensOverflow)
    })
}

mod tests;
