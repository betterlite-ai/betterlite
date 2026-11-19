#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section;

use super::{Raw, RegistryEntry, Settings};

const ONE_STORE: &str = "\
vector_store_registry:
  - vector_store_name: notes
    litellm_params:
      vector_store_id: vs_a
      custom_llm_provider: openai
      vector_store_description: d
      vector_store_metadata:
        tag: x
";

const ENVIRONMENT_REFERENCES: &str = "\
vector_store_registry:
  - vector_store_name: os.environ/REGISTRY_NAME
    litellm_params:
      vector_store_id: os.environ/REGISTRY_ID
      custom_llm_provider: bedrock
      filters:
        - os.environ/LISTED_LITERAL
        - nested: os.environ/NESTED_VALUE
";

const DUPLICATE_IDS: &str = "\
vector_store_registry:
  - vector_store_name: notes
    litellm_params:
      vector_store_id: vs_a
      custom_llm_provider: openai
  - vector_store_name: other
    litellm_params:
      vector_store_id: vs_a
      custom_llm_provider: openai
";

#[rstest]
#[case::absent("{}\n", Settings::default())]
#[case::null_section("vector_store_registry: null\n", Settings::default())]
#[case::empty_list("vector_store_registry: []\n", Settings::default())]
#[case::null_parameters(
    "vector_store_registry:\n  - vector_store_name: null\n    litellm_params: null\n",
    Settings {
        stores: vec![RegistryEntry {
            vector_store_name: None,
            litellm_params: None,
        }],
    }
)]
#[case::empty_strings(
    "vector_store_registry:\n  - vector_store_name: \"\"\n    litellm_params:\n      vector_store_id: \"\"\n      custom_llm_provider: \"\"\n",
    Settings {
        stores: vec![RegistryEntry {
            vector_store_name: Some(String::new()),
            litellm_params: Some(JsonObject::from([
                (
                    String::from("vector_store_id"),
                    Json::Text(String::new()),
                ),
                (
                    String::from("custom_llm_provider"),
                    Json::Text(String::new()),
                ),
            ])),
        }],
    }
)]
#[case::one_store(
    ONE_STORE,
    Settings {
        stores: vec![RegistryEntry {
            vector_store_name: Some("notes".to_owned()),
            litellm_params: Some(JsonObject::from([
                (String::from("vector_store_id"), Json::Text("vs_a".to_owned())),
                (
                    String::from("custom_llm_provider"),
                    Json::Text("openai".to_owned()),
                ),
                (
                    String::from("vector_store_description"),
                    Json::Text("d".to_owned()),
                ),
                (
                    String::from("vector_store_metadata"),
                    Json::Object(JsonObject::from([(
                        String::from("tag"),
                        Json::Text("x".to_owned()),
                    )])),
                ),
            ])),
        }],
    }
)]
#[case::duplicate_ids(
    DUPLICATE_IDS,
    Settings {
        stores: vec![
            RegistryEntry {
                vector_store_name: Some("notes".to_owned()),
                litellm_params: Some(JsonObject::from([
                    (String::from("vector_store_id"), Json::Text("vs_a".to_owned())),
                    (
                        String::from("custom_llm_provider"),
                        Json::Text("openai".to_owned()),
                    ),
                ])),
            },
            RegistryEntry {
                vector_store_name: Some("other".to_owned()),
                litellm_params: Some(JsonObject::from([
                    (String::from("vector_store_id"), Json::Text("vs_a".to_owned())),
                    (
                        String::from("custom_llm_provider"),
                        Json::Text("openai".to_owned()),
                    ),
                ])),
            },
        ],
    }
)]
fn loads_registry_config(#[case] source: &str, #[case] expected: Settings) {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .expect("vector_store_registry is a supported configuration key");
    let (raw, ignored) = parse_view::<Raw>(source).expect("the registry section parses");
    assert!(ignored.is_empty());
    let settings = Section::convert(raw, &BTreeMap::<String, String>::new())
        .expect("the registry section converts");
    assert_eq!(settings, expected);
}

#[test]
fn reads_registry_environment_references() {
    let lookup = BTreeMap::from([
        ("REGISTRY_NAME".to_owned(), "notes".to_owned()),
        ("REGISTRY_ID".to_owned(), "vs_a".to_owned()),
        ("NESTED_VALUE".to_owned(), "resolved".to_owned()),
    ]);
    let (raw, ignored) = parse_view::<Raw>(ENVIRONMENT_REFERENCES).expect("the section parses");
    assert!(ignored.is_empty());
    let settings = Section::convert(raw, &lookup).expect("the section converts");
    assert_eq!(
        settings,
        Settings {
            stores: vec![RegistryEntry {
                vector_store_name: Some("notes".to_owned()),
                litellm_params: Some(JsonObject::from([
                    (
                        String::from("vector_store_id"),
                        Json::Text("vs_a".to_owned())
                    ),
                    (
                        String::from("custom_llm_provider"),
                        Json::Text("bedrock".to_owned()),
                    ),
                    (
                        String::from("filters"),
                        Json::List(vec![
                            Json::Text("os.environ/LISTED_LITERAL".to_owned()),
                            Json::Object(JsonObject::from([(
                                String::from("nested"),
                                Json::Text("resolved".to_owned()),
                            )])),
                        ]),
                    ),
                ])),
            }],
        }
    );
}

#[test]
fn rejects_an_unset_registry_environment_reference() {
    let error = Section::convert(
        parse_view::<Raw>(ENVIRONMENT_REFERENCES)
            .expect("the section parses")
            .0,
        &BTreeMap::<String, String>::new(),
    )
    .expect_err("an unset environment variable names the field it belongs to");
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "REGISTRY_NAME"
                && path == "vector_store_registry.0.vector_store_name"
    ));
}
