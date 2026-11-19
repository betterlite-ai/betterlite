use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Number;
use serde_json::Value;

use super::{AppliedInputType, ModerationResponse, ModerationResponseError, ModerationResult};

// the SDK's dump keeps the provider's alias keys beside its field names, so the raw maps come first, typed
#[derive(Deserialize)]
pub(super) struct ProviderMaps {
    results: Vec<ResultMaps>,
}

#[derive(Deserialize)]
struct ResultMaps {
    categories: IndexMap<String, Option<bool>>,
    category_applied_input_types: IndexMap<String, Option<Vec<AppliedInputType>>>,
    category_scores: IndexMap<String, Number>,
}

// the alias each SDK field declares, which its dump repeats beside the field name
const ALIASES: [&str; 8] = [
    "harassment/threatening",
    "hate/threatening",
    "illicit/violent",
    "self-harm",
    "self-harm/instructions",
    "self-harm/intent",
    "sexual/minors",
    "violence/graphic",
];

// the SDK types seven categories as text-only and the other six as text or image
const IMAGE_CATEGORIES: [&str; 6] = [
    "self_harm",
    "self_harm_instructions",
    "self_harm_intent",
    "sexual",
    "violence",
    "violence_graphic",
];

impl ProviderMaps {
    // the typed record cannot say which score failed, so every score goes through B204's exact parser first
    pub(super) fn check_scores(&self) -> Result<(), ModerationResponseError> {
        for result in &self.results {
            result.check_scores()?;
        }
        Ok(())
    }

    // OpenAIModerationResponse dumps the SDK payload, which repeats every alias key the provider sent
    pub(super) fn keep_alias_keys(
        &self,
        response: &mut ModerationResponse,
    ) -> Result<(), ModerationResponseError> {
        for (result, raw) in response.results.iter_mut().zip(&self.results) {
            raw.keep_alias_keys(result)?;
        }
        Ok(())
    }
}

impl ResultMaps {
    fn check_scores(&self) -> Result<(), ModerationResponseError> {
        for (category, score) in &self.category_scores {
            parse_decimal_lexeme(&score.to_string()).map_err(|source| {
                ModerationResponseError::Decimal {
                    category: category.clone(),
                    source,
                }
            })?;
        }
        Ok(())
    }

    fn keep_alias_keys(
        &self,
        result: &mut ModerationResult,
    ) -> Result<(), ModerationResponseError> {
        let categories = self
            .categories
            .iter()
            .filter(|(alias, _)| ALIASES.contains(&alias.as_str()))
            .map(|(alias, flag)| (alias.clone(), flag.map_or(Value::Null, Value::Bool)));
        result.categories.extra.extend(categories);
        let input_types = self
            .category_applied_input_types
            .iter()
            .filter(|(alias, _)| ALIASES.contains(&alias.as_str()))
            .map(|(alias, inputs)| {
                serde_json::to_value(inputs)
                    .map(|value| (alias.clone(), value))
                    .map_err(|source| ModerationResponseError::Decode { source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        result
            .category_applied_input_types
            .extra
            .extend(input_types);
        let scores = self
            .category_scores
            .iter()
            .filter(|(alias, _)| ALIASES.contains(&alias.as_str()))
            .map(|(alias, score)| (alias.clone(), Value::Number(score.clone())));
        result.category_scores.extra.extend(scores);
        Ok(())
    }
}

pub(super) fn check_input_types(
    response: &ModerationResponse,
) -> Result<(), ModerationResponseError> {
    for result in &response.results {
        let types = &result.category_applied_input_types;
        let fields: [(&str, &Vec<AppliedInputType>); 13] = [
            ("harassment", &types.harassment),
            ("harassment_threatening", &types.harassment_threatening),
            ("hate", &types.hate),
            ("hate_threatening", &types.hate_threatening),
            ("illicit", &types.illicit),
            ("illicit_violent", &types.illicit_violent),
            ("self_harm", &types.self_harm),
            ("self_harm_instructions", &types.self_harm_instructions),
            ("self_harm_intent", &types.self_harm_intent),
            ("sexual", &types.sexual),
            ("sexual_minors", &types.sexual_minors),
            ("violence", &types.violence),
            ("violence_graphic", &types.violence_graphic),
        ];
        for (name, inputs) in fields {
            if inputs.contains(&AppliedInputType::Image) && !IMAGE_CATEGORIES.contains(&name) {
                return Err(ModerationResponseError::Decode {
                    source: serde_json::Error::custom(format!(
                        "the {name} applied input type is text only"
                    )),
                });
            }
        }
    }
    Ok(())
}
