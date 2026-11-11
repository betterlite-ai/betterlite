use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::classification::{ParameterClass, classify_parameter};
use super::{CallType, ChatParameters, ParameterError, ParameterMapping, ParameterPolicy};

pub(super) fn reject_nested_drop_paths(paths: &[String]) -> Result<(), ParameterError> {
    paths
        .iter()
        .find(|path| path.contains('.') || path.contains('['))
        .map_or(Ok(()), |path| {
            Err(ParameterError::NestedDropPath { path: path.clone() })
        })
}

pub(super) struct MappingState<'request> {
    allowed: BTreeSet<&'request str>,
    policy: &'request ParameterPolicy,
    mapping: ParameterMapping,
    unsupported: Vec<String>,
}

impl<'request> MappingState<'request> {
    pub(super) fn new(supported: &[&'static str], policy: &'request ParameterPolicy) -> Self {
        let mut allowed: BTreeSet<&'request str> = supported.iter().copied().collect();
        allowed.extend(policy.allowed_openai_params.iter().map(String::as_str));
        Self {
            allowed,
            policy,
            mapping: ParameterMapping::default(),
            unsupported: Vec::new(),
        }
    }

    pub(super) fn place(&mut self, name: String, value: Value, passthrough: bool) {
        if value.is_null() {
            return;
        }
        if self
            .policy
            .additional_drop_params
            .iter()
            .any(|dropped| dropped == &name)
        {
            self.mapping.dropped_fields.push(name);
        } else if passthrough {
            if self.drops_extra_body() {
                self.mapping.dropped_fields.push(name);
            } else {
                self.mapping.provider_fields.insert(name, value);
            }
        } else if self.allowed.contains(name.as_str()) {
            self.mapping.standard.insert(name, value);
        } else if is_unvalidated(&name, &value) || self.policy.drop_params {
            self.mapping.dropped_fields.push(name);
        } else {
            self.unsupported.push(name);
        }
    }

    pub(super) fn extra_body(
        &mut self,
        value: &Value,
        call_type: CallType,
    ) -> Result<(), ParameterError> {
        if value.is_null() {
            return Ok(());
        }
        let Value::Object(fields) = value else {
            return Err(ParameterError::NotAnObject {
                name: String::from("extra_body"),
            });
        };
        for (name, entry) in fields {
            if name == "model" || self.drops_extra_body() || self.drops_name(name) {
                self.mapping.dropped_fields.push(name.clone());
            } else if classify_parameter(name, call_type) == ParameterClass::Reserved {
                return Err(ParameterError::ReservedParameter { name: name.clone() });
            } else {
                self.mapping
                    .provider_fields
                    .entry(name.clone())
                    .or_insert_with(|| entry.clone());
            }
        }
        Ok(())
    }

    fn drops_extra_body(&self) -> bool {
        self.policy
            .additional_drop_params
            .iter()
            .any(|dropped| dropped == "extra_body")
    }

    fn drops_name(&self, name: &str) -> bool {
        self.policy
            .additional_drop_params
            .iter()
            .any(|dropped| dropped == name)
    }

    pub(super) fn finish(self, model: &str) -> Result<ParameterMapping, ParameterError> {
        if !self.unsupported.is_empty() {
            return Err(ParameterError::Unsupported {
                model: model.to_owned(),
                names: self.unsupported,
            });
        }
        let mut mapping = self.mapping;
        mapping.dropped_fields.sort();
        mapping.dropped_fields.dedup();
        Ok(mapping)
    }
}

fn is_unvalidated(name: &str, value: &Value) -> bool {
    matches!(name, "user" | "stream" | "stream_options" | "max_retries")
        || (name == "n" && value.as_i64() == Some(1))
}

pub(super) fn typed_parameters(
    parameters: &ChatParameters,
) -> Result<BTreeMap<String, Value>, ParameterError> {
    let encoded =
        serde_json::to_value(parameters).map_err(|source| ParameterError::Encode { source })?;
    serde_json::from_value(encoded).map_err(|source| ParameterError::Encode { source })
}
