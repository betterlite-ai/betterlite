#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;
use chrono::Utc;
use rstest::rstest;

use super::{
    CredentialLookup, CredentialLookupError, OperationKwargs, ensure_provider,
    prepare_registry_credentials,
};
use crate::api::vector_store_registry::store::{VectorStoreCredentials, VectorStoreRegistry};

fn object(fields: &[(&str, Json)]) -> JsonObject {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

struct ScriptedLookup {
    result: Result<OperationKwargs, CredentialLookupError>,
    calls: Mutex<Vec<String>>,
}

impl ScriptedLookup {
    fn answering(values: OperationKwargs) -> Self {
        Self {
            result: Ok(values),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            result: Err(CredentialLookupError::Unavailable {
                reason: "unavailable".to_owned(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl CredentialLookup for ScriptedLookup {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(vector_store_id.to_owned());
        self.result.clone()
    }
}

#[rstest]
#[case::null(None, "openai")]
#[case::empty(Some(""), "openai")]
#[case::azure(Some("azure"), "azure")]
#[case::prefixed(Some("openai/gpt-4o"), "openai/gpt-4o")]
#[case::unknown(Some("unknown"), "unknown")]
fn provider_defaults(#[case] input: Option<&str>, #[case] expected: &str) {
    assert_eq!(ensure_provider(input), expected);
}

#[rstest]
#[case::registry_values(
    object(&[
        ("api_key", text("registry")),
        ("api_base", text("new")),
        ("custom_llm_provider", text("azure")),
    ]),
    object(&[
        ("api_key", text("registry")),
        ("api_base", text("new")),
        ("timeout", number(7)),
        ("custom_llm_provider", text("azure")),
    ]),
)]
#[case::null_api_key(
    object(&[("api_key", Json::Null)]),
    object(&[
        ("api_key", Json::Null),
        ("api_base", text("old")),
        ("timeout", number(7)),
    ]),
)]
fn registry_overwrites_caller(#[case] overlay: JsonObject, #[case] expected: JsonObject) {
    let mut kwargs = OperationKwargs(object(&[
        ("api_key", text("caller")),
        ("api_base", text("old")),
        ("timeout", number(7)),
    ]));
    let lookup = ScriptedLookup::answering(OperationKwargs(overlay));
    prepare_registry_credentials("vs_a", &mut kwargs, Some(&lookup));
    assert_eq!(kwargs, OperationKwargs(expected));
    assert_eq!(lookup.calls(), vec!["vs_a".to_owned()]);
}

enum Registry {
    Absent,
    Empty,
    Failing,
}

#[rstest]
#[case::absent_registry(Registry::Absent, 0)]
#[case::empty_result(Registry::Empty, 1)]
#[case::lookup_error(Registry::Failing, 1)]
fn registry_failure_keeps_arguments(#[case] registry: Registry, #[case] expected_calls: usize) {
    let mut kwargs = OperationKwargs(object(&[
        ("api_key", text("caller")),
        ("timeout", number(7)),
    ]));
    let original = kwargs.clone();
    let lookup = match registry {
        Registry::Absent => None,
        Registry::Empty => Some(ScriptedLookup::answering(
            OperationKwargs(JsonObject::new()),
        )),
        Registry::Failing => Some(ScriptedLookup::failing()),
    };
    prepare_registry_credentials("vs_a", &mut kwargs, lookup.as_ref());
    assert_eq!(kwargs, original);
    assert_eq!(
        lookup.as_ref().map_or(0, |scripted| scripted.calls().len()),
        expected_calls
    );
}

struct StubCredentials;

impl VectorStoreCredentials for StubCredentials {
    fn credential_values(&self, credential_name: &str) -> JsonObject {
        if credential_name == "cred" {
            object(&[("api_key", text("registry"))])
        } else {
            JsonObject::new()
        }
    }
}

#[test]
fn registry_lookup_returns_credential_values() {
    let store = LiteLlmManagedVectorStore {
        vector_store_id: Some("vs_a".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        vector_store_name: None,
        vector_store_description: None,
        vector_store_metadata: None,
        created_at: None,
        updated_at: None,
        litellm_credential_name: Some("cred".to_owned()),
        litellm_params: None,
        team_id: None,
        user_id: None,
        is_config: None,
        extra: BTreeMap::new(),
    };
    let registry = VectorStoreRegistry {
        stores: vec![store],
        clock: Arc::new(Utc::now),
        credentials: Arc::new(StubCredentials),
    };
    let credentials = registry.lookup_credentials("vs_a").unwrap();
    assert_eq!(
        credentials,
        OperationKwargs(object(&[("api_key", text("registry"))]))
    );
    assert!(
        registry
            .lookup_credentials("vs_absent")
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn operation_kwargs_debug_hides_values() {
    let kwargs = OperationKwargs(object(&[
        ("api_key", text("sk-caller-secret")),
        ("api_base", text("https://old.example.test")),
    ]));
    let rendered = format!("{kwargs:?}");
    assert!(!rendered.contains("sk-caller-secret"));
    assert!(!rendered.contains("old.example.test"));
    assert!(rendered.contains("api_key"));
}
