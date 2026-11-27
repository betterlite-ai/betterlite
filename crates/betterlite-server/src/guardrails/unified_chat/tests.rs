#![cfg(test)]

pub(crate) mod support;

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::settings::JsonValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::{
    GuardrailFailure, ModifyResponseFailure, SensitiveDataRouteFailure,
};
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use indexmap::IndexMap;

use self::support::{FakeGuardrail, context, principal, texts_reply};
use super::{GuardrailToApply, failure_message, failure_parts, modify_response};

struct DefaultGuardrail;

impl GuardrailContract for DefaultGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        _request_data: &mut BTreeMap<String, GuardrailValue>,
        _input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        ready(Ok(inputs))
    }
}

fn modify_failure(message: &str) -> ModifyResponseFailure {
    ModifyResponseFailure {
        message: message.to_owned(),
        model: String::from("gpt-4o"),
        request_data: BTreeMap::new(),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        original_response: None,
    }
}

#[test]
fn helper_failure_parts_map_http_and_object_failures() {
    let http = GuardrailFailure::Http {
        status: 429,
        detail: String::from("quota"),
    };
    assert_eq!(
        failure_parts(&http),
        Some((429, JsonValue::String(String::from("quota"))))
    );
    let object = GuardrailFailure::HttpObject {
        status: 422,
        detail: IndexMap::from([(
            String::from("message"),
            GuardrailValue::Text(String::from("bad")),
        )]),
    };
    let (status, detail) = failure_parts(&object).unwrap();
    assert_eq!(status, 422);
    assert_eq!(detail.to_string(), r#"{"message":"bad"}"#);
}

#[test]
fn helper_failure_parts_map_modify_and_raised_failures() {
    let modify = GuardrailFailure::ModifyResponse(Box::new(modify_failure("blocked")));
    assert_eq!(
        failure_parts(&modify),
        Some((400, JsonValue::String(String::from("blocked"))))
    );
    let raised = GuardrailFailure::GuardrailRaised {
        guardrail_name: Some(String::from("guard")),
        message: String::from("denied"),
        wrap_default_message: false,
        status_code: 403,
        blocked_content: false,
    };
    assert_eq!(
        failure_parts(&raised),
        Some((403, JsonValue::String(String::from("denied"))))
    );
}

#[test]
fn helper_failure_parts_map_pii_and_skip_the_rest() {
    let pii = GuardrailFailure::BlockedPii {
        entity_type: String::from("EMAIL"),
        guardrail_name: Some(String::from("guard")),
        status_code: 400,
    };
    let (status, detail) = failure_parts(&pii).unwrap();
    assert_eq!(status, 400);
    assert!(detail.to_string().contains("EMAIL"));
    let route = GuardrailFailure::SensitiveDataRoute(Box::new(SensitiveDataRouteFailure {
        route_to_model: String::from("model"),
        session_id: String::from("session"),
        guardrail_name: None,
        detection_info: BTreeMap::new(),
        message: None,
        sticky_session_routing: false,
    }));
    assert!(failure_parts(&route).is_none());
    assert!(failure_parts(&GuardrailFailure::Other(String::from("boom"))).is_none());
}

#[test]
fn helper_failure_message_reads_the_detail_text() {
    let http = GuardrailFailure::Http {
        status: 429,
        detail: String::from("quota"),
    };
    assert_eq!(failure_message(&http), Some((429, String::from("quota"))));
    assert!(failure_message(&GuardrailFailure::Other(String::from("boom"))).is_none());
}

#[test]
fn helper_modify_response_only_matches_a_modify_failure() {
    let modify = GuardrailFailure::ModifyResponse(Box::new(modify_failure("blocked")));
    assert_eq!(
        modify_response(&modify).map(|details| details.message.clone()),
        Some(String::from("blocked"))
    );
    assert!(modify_response(&GuardrailFailure::Other(String::from("boom"))).is_none());
}

#[tokio::test]
async fn contract_apply_guardrail_reaches_the_guarded_guardrail() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&["masked"])));
    let applied = context(&fake, Some("guard"));
    let mut request_data = BTreeMap::new();
    let answer = applied
        .apply_guardrail(
            texts_reply(&["hi"]),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(answer.texts.as_ref().unwrap(), &[String::from("masked")]);
    assert_eq!(fake.calls().len(), 1);
}

#[test]
fn contract_reads_its_flags_from_the_guarded_guardrail() {
    let fake = Arc::new(FakeGuardrail::replying(texts_reply(&[])));
    let applied = context(&fake, Some("guard"));
    assert_eq!(
        applied.get_config_model(),
        Some(GuardrailValue::Text(String::from("model")))
    );
    assert_eq!(
        applied.get_supported_event_hooks(),
        Some(vec![GuardrailEventHooks::PostCall])
    );
    assert!(!applied.supports_tool_result_scan());
    assert!(applied.messages_cover_full_request());
}

#[test]
fn contract_keeps_the_guarded_guardrail_contract_defaults() {
    let applied = GuardrailToApply {
        guardrail: Arc::new(DefaultGuardrail),
        guardrail_name: None,
        principal: principal(),
        skip_flags: GuardrailSkipFlags::default(),
        global_flags: GlobalSkipFlags::default(),
    };
    assert!(applied.get_config_model().is_none());
    assert!(applied.get_supported_event_hooks().is_none());
    assert!(applied.supports_tool_result_scan());
    assert!(!applied.messages_cover_full_request());
}
