use betterlite_models::decisions::Json;
use betterlite_models::metadata::levels::ReasoningEffort;
use serde_json::Number;

use super::{ALPHA, BETA, Scenario, decimal, group_of, parse};
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::{Capability, ClientsideAuth, ModelGroupInfo};

#[test]
fn rejects_a_fractional_limit() {
    let mut scenario = Scenario::new(&[ALPHA]);
    scenario.metadata("a", r#"{"mode":"chat","tpm":1.5}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    assert!(matches!(
        scenario.group("g"),
        Err(GroupInfoError::Field { field: "tpm" })
    ));
}

// A two-deployment group with every wire field set to a value the aggregation alone would not produce.
fn decorated_group() -> ModelGroupInfo {
    let mut scenario = Scenario::new(&[ALPHA, BETA]);
    scenario.metadata("a", r#"{"mode":"chat"}"#);
    scenario.metadata("b", r#"{"mode":"chat"}"#);
    scenario.answer("openai/b846-alpha", "b846-alpha", "openai");
    scenario.answer("anthropic/b846-beta", "b846-beta", "anthropic");
    let mut group = group_of(&scenario, "g");
    group.input_cost_per_token = Some(decimal("0.5"));
    group.input_cost_per_pixel = Some(decimal("0.25"));
    group.tpm = Some(7);
    group.capabilities.insert(Capability::Vision);
    group.supported_reasoning_efforts = Some(vec![ReasoningEffort::Low]);
    group.supported_openai_params = Some(vec!["temperature".to_owned()]);
    group.clientside_auth = Some(vec![
        ClientsideAuth::Name("temperature".to_owned()),
        ClientsideAuth::Item(parse(r#"{"api_base":"https://b846"}"#)),
    ]);
    group
}

#[test]
fn wire_identity_and_limits() {
    let wire = decorated_group().to_json();
    assert_eq!(wire.get("model_group"), Some(&Json::Text("g".to_owned())));
    assert_eq!(
        wire.get("providers"),
        Some(&Json::List(vec![
            Json::Text("openai".to_owned()),
            Json::Text("anthropic".to_owned()),
        ]))
    );
    assert_eq!(wire.get("max_input_tokens"), Some(&Json::Null));
    assert_eq!(wire.get("tpm"), Some(&Json::Number(Number::from(7_i64))));
    assert_eq!(wire.get("mode"), Some(&Json::Text("chat".to_owned())));
    assert_eq!(wire.get("supports_vision"), Some(&Json::Bool(true)));
    assert_eq!(wire.get("supports_url_context"), Some(&Json::Bool(false)));
}

#[test]
fn wire_costs_and_params() {
    let wire = decorated_group().to_json();
    assert_eq!(
        wire.get("input_cost_per_token"),
        Some(&Json::Number(Number::from_f64(0.5).expect("finite")))
    );
    assert_eq!(
        wire.get("supported_reasoning_efforts"),
        Some(&Json::List(vec![Json::Text("low".to_owned())]))
    );
    assert_eq!(
        wire.get("supported_openai_params"),
        Some(&Json::List(vec![Json::Text("temperature".to_owned())]))
    );
    assert_eq!(
        wire.get("configurable_clientside_auth_params"),
        Some(&Json::List(vec![
            Json::Text("temperature".to_owned()),
            Json::Object(parse(r#"{"api_base":"https://b846"}"#)),
        ]))
    );
}
