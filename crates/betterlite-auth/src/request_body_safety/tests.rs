#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::provider_url_destination::pattern::AllowedDestination;
use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_models::model_registration::JsonObject;

use super::{FormMetadataExtractor, ModelGrantLookup, ModelGrants, RequestBodySafety, SafetyError};

mod deviations;
mod fallback;
mod grants;
mod metadata;
mod replay;
mod scan;

#[derive(Debug, Default)]
pub(super) struct Grants(BTreeMap<String, Option<Vec<ClientsideParamGrant>>>);

impl Grants {
    pub(super) fn exact(entries: &[(&str, &[&str])]) -> Self {
        let mut map = BTreeMap::new();
        for (model, names) in entries {
            let items = names
                .iter()
                .map(|name| ClientsideParamGrant::Exact((*name).to_owned()))
                .collect();
            map.insert((*model).to_owned(), Some(items));
        }
        Self(map)
    }

    pub(super) fn unset(model: &str) -> Self {
        let mut map = BTreeMap::new();
        map.insert(model.to_owned(), None);
        Self(map)
    }
}

impl ModelGrantLookup for Grants {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        match self.0.get(model_group) {
            None => ModelGrants::Absent,
            Some(None) => ModelGrants::Unset,
            Some(Some(items)) => ModelGrants::Granted(items),
        }
    }
}

// Stand-in for B1101's extract_nested_form_metadata: only the top-level names matter.
#[derive(Debug)]
pub(super) struct Extract;

impl FormMetadataExtractor for Extract {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for key in form_data.keys() {
            let Some(path) = key.strip_prefix(prefix) else {
                continue;
            };
            let first = path
                .trim_end_matches(']')
                .split("][")
                .next()
                .unwrap_or_default();
            if !first.is_empty() && !names.iter().any(|name| name == first) {
                names.push(first.to_owned());
            }
        }
        names
    }
}

pub(super) struct Fixture {
    pub(super) grants: Grants,
    pub(super) allow: bool,
    pub(super) hosts: Vec<AllowedDestination>,
    pub(super) max_fallbacks: i64,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            grants: Grants::default(),
            allow: false,
            hosts: Vec::new(),
            max_fallbacks: 5,
        }
    }
}

impl Fixture {
    pub(super) fn check(&self, body: &JsonObject, model: &str) -> Result<(), SafetyError> {
        super::is_request_body_safe(&RequestBodySafety {
            body,
            model,
            allow_client_side_credentials: self.allow,
            allowed_hosts: &self.hosts,
            grants: &self.grants,
            metadata: &Extract,
            max_fallbacks: self.max_fallbacks,
        })
    }

    pub(super) fn check_json(&self, text: &str, model: &str) -> Result<(), SafetyError> {
        self.check(&body(text), model)
    }
}

pub(super) fn body(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture body is valid JSON")
}

pub(super) fn passes(fixture: &Fixture, text: &str) {
    fixture
        .check_json(text, "model")
        .unwrap_or_else(|error| panic!("{text} should be admitted, got {error}"));
}

pub(super) fn denied(fixture: &Fixture, text: &str) -> SafetyError {
    fixture
        .check_json(text, "model")
        .expect_err("the body is refused")
}

pub(super) fn denied_param(fixture: &Fixture, text: &str) -> String {
    match fixture.check_json(text, "model") {
        Err(SafetyError::Banned { param }) => param,
        other => panic!("{text} should be refused for a banned field, got {other:?}"),
    }
}
