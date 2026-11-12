use betterlite_models::catalog::CatalogEntry;
use betterlite_models::token_cost::GenericCostResult;
use rust_decimal::Decimal;

use super::delegate::GenericTokenPricing;
use super::error::GooglePricingError;
use super::token::{TokenRequest, cost_per_token};
use super::usage::GoogleUsage;

const FIXED_CHARACTER_MODELS: [&str; 3] = ["gemini-1.0-pro", "gemini-pro", "gemini-2"];

pub struct CharacterRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub prompt_characters: Option<Decimal>,
    pub completion_characters: Option<Decimal>,
    pub service_tier: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
}

pub fn cost_per_character<P>(
    pricing: &P,
    record: &CatalogEntry,
    usage: &GoogleUsage,
    request: &CharacterRequest<'_>,
) -> Result<GenericCostResult, GooglePricingError>
where
    P: GenericTokenPricing,
{
    let prompt_cost = character_side(pricing, record, usage, request, CharacterSide::Prompt)?;
    let completion_cost =
        character_side(pricing, record, usage, request, CharacterSide::Completion)?;
    let uplift = pricing.vertex_endpoint_uplift(record, request.vertex_location);
    Ok(GenericCostResult {
        prompt_cost: prompt_cost
            .checked_mul(uplift)
            .ok_or(GooglePricingError::Overflow {
                field: "prompt character",
            })?,
        completion_cost: completion_cost.checked_mul(uplift).ok_or(
            GooglePricingError::Overflow {
                field: "completion character",
            },
        )?,
    })
}

#[derive(Clone, Copy)]
enum CharacterSide {
    Prompt,
    Completion,
}

fn character_side<P>(
    pricing: &P,
    record: &CatalogEntry,
    usage: &GoogleUsage,
    request: &CharacterRequest<'_>,
    side: CharacterSide,
) -> Result<Decimal, GooglePricingError>
where
    P: GenericTokenPricing,
{
    let requested = match side {
        CharacterSide::Prompt => request.prompt_characters,
        CharacterSide::Completion => request.completion_characters,
    };
    let Some(characters) = requested else {
        return token_side(pricing, record, usage, request, side);
    };
    let above =
        characters
            .checked_mul(Decimal::from(4_i64))
            .ok_or(GooglePricingError::Overflow {
                field: "character threshold",
            })?
            > Decimal::from(128_000_i64);
    let dynamic = above && !FIXED_CHARACTER_MODELS.contains(&request.model);
    let prices = &record.prices.values;
    let configured = match (side, dynamic) {
        (CharacterSide::Prompt, false) => prices.input.input_character,
        (CharacterSide::Prompt, true) => prices.input.input_character_over_128k,
        (CharacterSide::Completion, false) => prices.output.output_character,
        (CharacterSide::Completion, true) => prices.output.output_character_over_128k,
    };
    let Some(rate) = configured else {
        return token_side(pricing, record, usage, request, side);
    };
    // the source bills the above-128k output rate against completion tokens, not completion characters
    let quantity = match (side, dynamic) {
        (CharacterSide::Completion, true) => Decimal::from(usage.tokens.completion_tokens),
        (CharacterSide::Prompt, _) | (CharacterSide::Completion, false) => characters,
    };
    quantity
        .checked_mul(rate)
        .ok_or(GooglePricingError::Overflow { field: "character" })
}

// the source calls cost_per_token without a vertex location here and applies the uplift once to both totals
fn token_side<P>(
    pricing: &P,
    record: &CatalogEntry,
    usage: &GoogleUsage,
    request: &CharacterRequest<'_>,
    side: CharacterSide,
) -> Result<Decimal, GooglePricingError>
where
    P: GenericTokenPricing,
{
    let token_request = TokenRequest {
        model: request.model,
        custom_llm_provider: request.custom_llm_provider,
        service_tier: request.service_tier,
        vertex_location: None,
    };
    let cost = cost_per_token(pricing, record, usage, &token_request)?;
    Ok(match side {
        CharacterSide::Prompt => cost.prompt_cost,
        CharacterSide::Completion => cost.completion_cost,
    })
}
