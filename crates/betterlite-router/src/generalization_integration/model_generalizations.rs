use betterlite_models::catalog_generalizations::{
    CatalogGeneralizations, ModelInfo, decode_generalization_rules,
};

use crate::generalization_integration::bounds::{bounded, declared_pattern};
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

impl ModelGeneralizations {
    #[must_use]
    pub fn install(bytes: &[u8]) -> Result<Self, GeneralizationError> {
        let mut kept = Vec::new();
        let mut warnings = Vec::new();
        for rule in decode_generalization_rules(bytes)? {
            match declared_pattern(&rule) {
                Some(pattern) if pattern.len() > MAX_PATTERN_BYTES => {
                    warnings.push(GeneralizationWarning {
                        kind: WarningKind::PatternTooLarge,
                        pattern,
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
        if !model.is_empty() {
            for rule in &self.engine.routing {
                match rule.pattern.is_match(model) {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(_) => warnings.push(limit_warning(rule.pattern.as_str())),
                }
            }
        }
        Found {
            answer: self.engine.match_routing(model).map(str::to_owned),
            warnings,
        }
    }

    #[must_use]
    pub fn capabilities(&self, model: &str) -> Found<ModelInfo> {
        let mut warnings = Vec::new();
        if !model.is_empty() {
            for rule in &self.engine.capabilities {
                if rule.pattern.is_match(model).is_err() {
                    warnings.push(limit_warning(rule.pattern.as_str()));
                }
            }
        }
        Found {
            answer: self.engine.match_capabilities(model),
            warnings,
        }
    }

    #[must_use]
    pub fn fill_missing(&self, model: &str, provider: &str) -> Found<ModelInfo> {
        let mut warnings = Vec::new();
        if !model.is_empty() && !provider.is_empty() {
            for rule in &self.engine.capabilities {
                let listed = rule.fill_missing_for_providers.contains(provider);
                if listed && rule.pattern.is_match(model).is_err() {
                    warnings.push(limit_warning(rule.pattern.as_str()));
                }
            }
        }
        Found {
            answer: self.engine.match_fill_missing(model, provider),
            warnings,
        }
    }
}

// the call-site merge: a value the exact entry already has wins, explicit null and false included
pub fn fill_absent(exact: &mut ModelInfo, proposed: ModelInfo) {
    for (key, value) in proposed {
        exact.entry(key).or_insert(value);
    }
}
