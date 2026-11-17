use std::collections::BTreeMap;

use betterlite_models::catalog_generalizations::{
    CapabilityRule, CatalogGeneralizations, ModelInfo, decode_generalization_rules,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::value::RawValue;

use crate::generalization_integration::bounds::bounded;
use crate::generalization_integration::error::{GeneralizationError, GeneralizationWarning};
use crate::tag_matchers::patterns::{MAX_PATTERN_BYTES, WarningKind};

mod tests;

#[derive(Debug, Clone)]
pub struct Found<T> {
    pub answer: Option<T>,
    pub warnings: Vec<GeneralizationWarning>,
}

// the B2229 engine installed from rule bytes, with every compiled pattern inside the task's bound
#[derive(Debug, Default)]
pub struct ModelGeneralizations {
    pub engine: CatalogGeneralizations,
    pub warnings: Vec<GeneralizationWarning>,
    pub engine_warnings: Vec<String>,
}

fn limit_warning(pattern: &str) -> GeneralizationWarning {
    GeneralizationWarning {
        kind: WarningKind::MatchLimitExceeded,
        pattern: pattern.to_owned(),
    }
}

// the engine skips a capability rule whose search fails, so this scan reports it in the same order
fn capability_warnings(
    rules: &[CapabilityRule],
    model: &str,
    provider: Option<&str>,
) -> Vec<GeneralizationWarning> {
    let mut warnings = Vec::new();
    if model.is_empty() || provider.is_some_and(str::is_empty) {
        return warnings;
    }
    for rule in rules {
        let wanted = provider.is_none_or(|name| rule.fill_missing_for_providers.contains(name));
        if wanted && rule.pattern.is_match(model).is_err() {
            warnings.push(limit_warning(rule.pattern.as_str()));
        }
    }
    warnings
}

impl ModelGeneralizations {
    pub fn install(bytes: &[u8]) -> Result<Self, GeneralizationError> {
        let mut kept = Vec::new();
        let mut warnings = Vec::new();
        for rule in decode_generalization_rules(bytes)? {
            let pattern: Option<String> =
                serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(rule.get())
                    .ok()
                    .and_then(|mut fields| fields.remove("pattern"))
                    .and_then(|value| serde_json::from_str(value.get()).ok());
            match pattern {
                Some(found) if found.len() > MAX_PATTERN_BYTES => {
                    warnings.push(GeneralizationWarning {
                        kind: WarningKind::PatternTooLarge,
                        pattern: found,
                    });
                }
                _ => kept.push(rule),
            }
        }
        let (mut engine, engine_warnings) = CatalogGeneralizations::install(kept);
        for rule in &mut engine.routing {
            if let Some(regex) = bounded(rule.pattern.as_str()) {
                rule.pattern = regex;
            }
        }
        for rule in &mut engine.capabilities {
            if let Some(regex) = bounded(rule.pattern.as_str()) {
                rule.pattern = regex;
            }
        }
        Ok(Self {
            engine,
            warnings,
            engine_warnings: engine_warnings.iter().map(ToString::to_string).collect(),
        })
    }

    pub fn reload(&mut self, bytes: &[u8]) -> Result<(), GeneralizationError> {
        *self = Self::install(bytes)?;
        Ok(())
    }

    // rules lose to an exact entry the caller already matched, so the exact provider skips the rule scan
    #[must_use]
    pub fn provider(&self, model: &str, exact: Option<&str>) -> Found<String> {
        if let Some(provider) = exact.filter(|provider| !provider.is_empty()) {
            return Found {
                answer: Some(provider.to_owned()),
                warnings: Vec::new(),
            };
        }
        self.routing(model)
    }

    #[must_use]
    pub fn routing(&self, model: &str) -> Found<String> {
        let mut warnings = Vec::new();
        for rule in &self.engine.routing {
            if model.is_empty() {
                break;
            }
            match rule.pattern.is_match(model) {
                Ok(true) => break,
                Ok(false) => (),
                Err(_) => warnings.push(limit_warning(rule.pattern.as_str())),
            }
        }
        Found {
            answer: self.engine.match_routing(model).map(str::to_owned),
            warnings,
        }
    }

    #[must_use]
    pub fn capabilities(&self, model: &str) -> Found<ModelInfo> {
        Found {
            answer: self.engine.match_capabilities(model),
            warnings: capability_warnings(&self.engine.capabilities, model, None),
        }
    }

    #[must_use]
    pub fn fill_missing(&self, model: &str, provider: &str) -> Found<ModelInfo> {
        Found {
            answer: self.engine.match_fill_missing(model, provider),
            warnings: capability_warnings(&self.engine.capabilities, model, Some(provider)),
        }
    }
}

// the call-site merge: a value the exact entry already has wins, explicit null and false included
pub fn fill_absent(exact: &mut JsonObject, proposed: ModelInfo) {
    for (key, value) in proposed {
        // the engine keeps a rule's model_info as JSON text, which always reads back into a value
        let Ok(json) = serde_json::from_str::<Json>(value.get()) else {
            continue;
        };
        exact.entry(key).or_insert(json);
    }
}
