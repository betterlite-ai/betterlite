#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::apply;
use crate::context::{self, RouteStage};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::generalization_integration::model_generalizations::ModelGeneralizations;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::services::RouteServices;
use crate::router_core::tests::support::{
    NoLimits, NoServices, Providers, route_context, route_request,
};
use crate::static_eligibility::request::RequestFacts;

const ROUTING: &str = r#"[{"pattern":"^acme-","model_info":{"litellm_provider":"acme"}}]"#;
const CAPABILITIES: &str =
    r#"[{"pattern":"^mystery-","model_info":{"supports_web_search":false,"max_tokens":100}}]"#;
const FILL_MISSING: &str = r#"[{"pattern":"^gemini-","model_info":{"supports_pdf":true},"fill_missing_for_providers":["vertex_ai"]}]"#;
const OPENAI_FILL: &str = r#"[{"pattern":"^gemini-","model_info":{"supports_pdf":true},"fill_missing_for_providers":["openai"]}]"#;
const ROUTE_THEN_FILL: &str = r#"[{"pattern":"^gemini-","model_info":{"litellm_provider":"azure"}},{"pattern":"^gemini-","model_info":{"supports_pdf":true},"fill_missing_for_providers":["vertex_ai"]}]"#;
const SLOW_RULE: &str =
    r#"[{"pattern":"(?i)(a|b|ab)*(?>c)","model_info":{"litellm_provider":"steps"}}]"#;
const SLOW_MODEL: &str = "abababababababababababab";

struct Generalized(ModelGeneralizations);

impl RouteServices for Generalized {
    fn model_generalizations(&self) -> Option<&ModelGeneralizations> {
        Some(&self.0)
    }
}

fn installed(rules: &str) -> Generalized {
    Generalized(ModelGeneralizations::install(rules.as_bytes()).unwrap())
}

fn spec(model: &str, provider: Option<&str>, model_info: JsonObject) -> DeploymentSpec {
    let mut litellm_params = JsonObject::new();
    litellm_params.insert(String::from("model"), Json::Text(model.to_owned()));
    if let Some(provider) = provider {
        litellm_params.insert(
            String::from("custom_llm_provider"),
            Json::Text(provider.to_owned()),
        );
    }
    DeploymentSpec {
        model_name: String::from("gpt-4o"),
        litellm_params,
        model_info,
        extra: JsonObject::new(),
    }
}

async fn filled_by(services: &dyn RouteServices, spec: DeploymentSpec) -> DeploymentSpec {
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, services, &limits, &params);
    let candidates = CandidateSet {
        deployments: vec![spec],
    };
    let filled = apply(candidates, &route).await.unwrap();
    filled.deployments.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_router_without_generalizations_keeps_the_pool() {
    assert!(NoServices.model_generalizations().is_none());
    let original = spec("acme-1", None, JsonObject::new());
    let filled = filled_by(&NoServices, original.clone()).await;
    assert_eq!(filled, original);
}

#[tokio::test]
async fn a_routing_rule_supplies_a_missing_provider() {
    let service = installed(ROUTING);
    let filled = filled_by(&service, spec("acme-1", None, JsonObject::new())).await;
    assert_eq!(
        filled.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("acme")))
    );
}

#[tokio::test]
async fn an_exact_provider_skips_the_routing_rules() {
    let service = installed(ROUTING);
    let filled = filled_by(&service, spec("acme-1", Some("openai"), JsonObject::new())).await;
    assert!(filled.model_info.get("litellm_provider").is_none());
}

#[tokio::test]
async fn an_empty_provider_still_takes_the_routing_rule() {
    let service = installed(ROUTING);
    let filled = filled_by(&service, spec("acme-1", Some(""), JsonObject::new())).await;
    assert_eq!(
        filled.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("acme")))
    );
}

#[tokio::test]
async fn capability_rules_fill_an_unknown_models_missing_fields() {
    let service = installed(CAPABILITIES);
    let filled = filled_by(&service, spec("mystery-1", None, JsonObject::new())).await;
    assert_eq!(
        filled.model_info.get("supports_web_search"),
        Some(&Json::Bool(false))
    );
    assert!(matches!(
        filled.model_info.get("max_tokens"),
        Some(Json::Number(number)) if number.as_u64() == Some(100)
    ));
}

#[tokio::test]
async fn a_fill_rule_applies_only_for_its_provider() {
    let service = installed(FILL_MISSING);
    let filled = filled_by(
        &service,
        spec("gemini-2", Some("vertex_ai"), JsonObject::new()),
    )
    .await;
    assert_eq!(
        filled.model_info.get("supports_pdf"),
        Some(&Json::Bool(true))
    );
    let without_fill = filled_by(
        &service,
        spec("gemini-2", Some("openai"), JsonObject::new()),
    )
    .await;
    assert!(without_fill.model_info.get("supports_pdf").is_none());
}

#[tokio::test]
async fn the_deployment_keeps_the_values_it_declares() {
    let service = installed(CAPABILITIES);
    let mut model_info = JsonObject::new();
    model_info.insert(String::from("supports_web_search"), Json::Bool(true));
    model_info.insert(String::from("max_tokens"), Json::Null);
    let filled = filled_by(&service, spec("mystery-1", None, model_info)).await;
    assert_eq!(
        filled.model_info.get("supports_web_search"),
        Some(&Json::Bool(true))
    );
    assert_eq!(filled.model_info.get("max_tokens"), Some(&Json::Null));
}

#[tokio::test]
async fn an_empty_model_info_provider_falls_to_the_params_provider() {
    let service = installed(ROUTE_THEN_FILL);
    let mut model_info = JsonObject::new();
    model_info.insert(String::from("litellm_provider"), Json::Text(String::new()));
    let filled = filled_by(&service, spec("gemini-2", Some("vertex_ai"), model_info)).await;
    assert_eq!(
        filled.model_info.get("supports_pdf"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        filled.model_info.get("litellm_provider"),
        Some(&Json::Text(String::new()))
    );
}

#[tokio::test]
async fn the_model_info_provider_wins_over_the_params_provider() {
    let service = installed(OPENAI_FILL);
    let mut model_info = JsonObject::new();
    model_info.insert(
        String::from("litellm_provider"),
        Json::Text(String::from("openai")),
    );
    let filled = filled_by(&service, spec("gemini-2", Some("vertex_ai"), model_info)).await;
    assert_eq!(
        filled.model_info.get("supports_pdf"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        filled.model_info.get("litellm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}

#[tokio::test]
async fn a_rule_over_the_match_limit_is_skipped() {
    let service = installed(SLOW_RULE);
    let filled = filled_by(&service, spec(SLOW_MODEL, None, JsonObject::new())).await;
    assert!(filled.model_info.get("litellm_provider").is_none());
}

#[tokio::test]
async fn the_router_runs_the_stage_when_rules_are_installed() {
    let service = installed(CAPABILITIES);
    let facts = RequestFacts::default();
    let request = route_request("gpt-4o", &facts);
    let (limits, params) = (NoLimits, Providers::Unknown);
    let route = route_context(&request, &service, &limits, &params);
    let candidates = CandidateSet {
        deployments: vec![spec("mystery-1", None, JsonObject::new())],
    };
    let ran = context::run(RouteStage::GeneralizationIntegration, candidates, &route)
        .await
        .unwrap();
    assert_eq!(
        ran.deployments[0].model_info.get("supports_web_search"),
        Some(&Json::Bool(false))
    );
}
