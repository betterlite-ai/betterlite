#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde::Serialize;

use super::{CatalogProviderNames, ProviderNames, call_surface, destination_host, model_provider};

struct Fixed {
    provider: Option<&'static str>,
    model: &'static str,
    api_base: Option<&'static str>,
    api_key: Option<&'static str>,
}

impl ProviderNames for Fixed {
    fn provider_for(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&str>,
    ) -> Option<String> {
        let matched = model == self.model && api_base == self.api_base && api_key == self.api_key;
        self.provider.filter(|_| matched).map(str::to_owned)
    }
}

enum Scenario {
    Provider {
        request: &'static str,
        model: Option<&'static str>,
        fixed: Fixed,
    },
    Destination(&'static str),
    Surface {
        logging: Option<&'static str>,
        request: &'static str,
    },
    Catalog {
        model: &'static str,
        api_base: Option<&'static str>,
    },
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    match parsed(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("expected an object")
        }
    }
}

fn as_json<T>(value: &T) -> Json
where
    T: Serialize,
{
    serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap()
}

fn project(scenario: &Scenario) -> Json {
    match scenario {
        Scenario::Provider {
            request,
            model,
            fixed,
        } => as_json(&model_provider(&object(request), *model, fixed)),
        Scenario::Destination(request) => as_json(&destination_host(&object(request))),
        Scenario::Surface { logging, request } => {
            as_json(&call_surface(*logging, &object(request)))
        }
        Scenario::Catalog { model, api_base } => {
            let catalog = ModelCatalog::default();
            let known_model_names = BTreeSet::new();
            let custom_providers = BTreeSet::new();
            let environment = BTreeMap::new();
            let registry = JSONProviderRegistry::load();
            let names = CatalogProviderNames {
                catalog: &catalog,
                known_model_names: &known_model_names,
                custom_providers: &custom_providers,
                environment: &environment,
                registry: &registry,
            };
            as_json(&names.provider_for(model, *api_base, None))
        }
    }
}

#[rstest]
#[case::provider_from_request_custom_llm_provider(
    Scenario::Provider {
        request: r#"{"custom_llm_provider":"cohere_chat","model":"command-r"}"#,
        model: Some("command-r"),
        fixed: Fixed { provider: Some("ignored"), model: "command-r", api_base: None, api_key: None },
    },
    parsed(r#""cohere_chat""#)
)]
#[case::provider_from_litellm_params_custom_llm_provider(
    Scenario::Provider {
        request: r#"{"litellm_params":{"custom_llm_provider":"anthropic"}}"#,
        model: None,
        fixed: Fixed { provider: Some("ignored"), model: "", api_base: None, api_key: None },
    },
    parsed(r#""anthropic""#)
)]
#[case::provider_falls_back_to_the_model(
    Scenario::Provider {
        request: r#"{"api_base":"https://base.example/v1","api_key":"sk-1"}"#,
        model: Some("openai/gpt-4o"),
        fixed: Fixed {
            provider: Some("openai"),
            model: "openai/gpt-4o",
            api_base: Some("https://base.example/v1"),
            api_key: Some("sk-1"),
        },
    },
    parsed(r#""openai""#)
)]
#[case::provider_falls_back_to_litellm_params_credentials(
    Scenario::Provider {
        request: r#"{"litellm_params":{"api_base":"https://params.example/v1","api_key":"sk-2"}}"#,
        model: Some("openai/gpt-4o"),
        fixed: Fixed {
            provider: Some("openai"),
            model: "openai/gpt-4o",
            api_base: Some("https://params.example/v1"),
            api_key: Some("sk-2"),
        },
    },
    parsed(r#""openai""#)
)]
#[case::provider_request_credentials_win(
    Scenario::Provider {
        request: r#"{"api_base":"https://request.example/v1","litellm_params":{"api_base":"https://params.example/v1"}}"#,
        model: Some("openai/gpt-4o"),
        fixed: Fixed {
            provider: Some("openai"),
            model: "openai/gpt-4o",
            api_base: Some("https://request.example/v1"),
            api_key: None,
        },
    },
    parsed(r#""openai""#)
)]
#[case::provider_ignores_non_string_credentials(
    Scenario::Provider {
        request: r#"{"api_base":5,"api_key":false,"litellm_params":{"api_base":"https://params.example/v1","api_key":"sk-2"}}"#,
        model: Some("openai/gpt-4o"),
        fixed: Fixed {
            provider: Some("openai"),
            model: "openai/gpt-4o",
            api_base: Some("https://params.example/v1"),
            api_key: Some("sk-2"),
        },
    },
    parsed(r#""openai""#)
)]
#[case::provider_ignores_a_non_string_declaration(
    Scenario::Provider {
        request: r#"{"custom_llm_provider":5,"model":"openai/gpt-4o"}"#,
        model: Some("openai/gpt-4o"),
        fixed: Fixed {
            provider: Some("openai"),
            model: "openai/gpt-4o",
            api_base: None,
            api_key: None,
        },
    },
    parsed(r#""openai""#)
)]
#[case::provider_without_a_model_is_absent(
    Scenario::Provider {
        request: "{}",
        model: None,
        fixed: Fixed { provider: Some("ignored"), model: "", api_base: None, api_key: None },
    },
    parsed("null")
)]
#[case::provider_resolution_failure_is_absent(
    Scenario::Provider {
        request: r#"{"model":"unknown-model-xyz"}"#,
        model: Some("unknown-model-xyz"),
        fixed: Fixed {
            provider: None,
            model: "unknown-model-xyz",
            api_base: None,
            api_key: None,
        },
    },
    parsed("null")
)]
#[case::destination_from_userinfo_and_port(
    Scenario::Destination(r#"{"api_base":"https://user:secret@api.example.com:8443/v1"}"#),
    parsed(r#""api.example.com""#)
)]
#[case::destination_from_litellm_params(
    Scenario::Destination(r#"{"litellm_params":{"api_base":"https://params.example:9000/v1"}}"#),
    parsed(r#""params.example""#)
)]
#[case::destination_without_a_scheme_is_absent(
    Scenario::Destination(r#"{"api_base":"api.example.com/v1"}"#),
    parsed("null")
)]
#[case::destination_ignores_a_non_string_base(
    Scenario::Destination(r#"{"api_base":5}"#),
    parsed("null")
)]
#[case::destination_rejects_a_malformed_ipv6_host(
    Scenario::Destination(r#"{"api_base":"https://[::1/api"}"#),
    parsed("null")
)]
#[case::surface_prefers_the_logging_object(
    Scenario::Surface {
        logging: Some("acompletion"),
        request: r#"{"call_type":"completion"}"#,
    },
    parsed(r#""acompletion""#)
)]
#[case::surface_falls_back_to_the_request_call_type(
    Scenario::Surface {
        logging: None,
        request: r#"{"call_type":"completion"}"#,
    },
    parsed(r#""completion""#)
)]
#[case::surface_falls_back_to_litellm_call_type(
    Scenario::Surface {
        logging: Some(""),
        request: r#"{"call_type":"","litellm_call_type":"text_completion"}"#,
    },
    parsed(r#""text_completion""#)
)]
#[case::surface_without_a_call_type_is_unknown(
    Scenario::Surface {
        logging: None,
        request: "{}",
    },
    parsed(r#""unknown""#)
)]
#[case::catalog_resolves_a_prefixed_model(
    Scenario::Catalog {
        model: "openai/gpt-4o",
        api_base: None,
    },
    parsed(r#""openai""#)
)]
#[case::catalog_uses_the_api_base(
    Scenario::Catalog {
        model: "llama3-8b-8192",
        api_base: Some("https://api.groq.com/openai/v1"),
    },
    parsed(r#""groq""#)
)]
#[case::catalog_unknown_model_is_absent(
    Scenario::Catalog {
        model: "unknown-model-xyz",
        api_base: None,
    },
    parsed("null")
)]
fn straiker_webhook_context_tests_behavior(#[case] scenario: Scenario, #[case] expected: Json) {
    assert_eq!(project(&scenario), expected);
}
