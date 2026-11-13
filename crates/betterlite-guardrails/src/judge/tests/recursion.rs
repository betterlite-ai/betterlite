use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use crate::judge::hooks::{
    GuardrailCallData, GuardrailEventHook, INTERNAL_CALL_ORIGIN, JUDGE_CALL_ORIGIN,
    is_logged_judge_call, should_run_guardrail,
};

#[derive(Debug, Clone, Copy)]
enum Data {
    Marker,
    MissingParams,
    MissingMetadata,
    NonObjectParams,
    NonObjectMetadata,
    WrongOrigin,
    EmptyMetadata,
}

fn object(entries: &[(&str, GuardrailValue)]) -> GuardrailValue {
    GuardrailValue::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn call_data(data: Data) -> GuardrailCallData {
    let litellm_params = match data {
        Data::Marker => object(&[(
            "metadata",
            object(&[(
                INTERNAL_CALL_ORIGIN,
                GuardrailValue::Text(JUDGE_CALL_ORIGIN.to_owned()),
            )]),
        )]),
        Data::MissingParams => return GuardrailCallData::default(),
        Data::MissingMetadata => {
            object(&[("user_api_key", GuardrailValue::Text("hash".to_owned()))])
        }
        Data::NonObjectParams => GuardrailValue::Text("not a mapping".to_owned()),
        Data::NonObjectMetadata => object(&[(
            "metadata",
            GuardrailValue::Array(vec![GuardrailValue::Text("not a mapping".to_owned())]),
        )]),
        Data::WrongOrigin => object(&[(
            "metadata",
            object(&[(
                INTERNAL_CALL_ORIGIN,
                GuardrailValue::Text("autorouter_classifier".to_owned()),
            )]),
        )]),
        Data::EmptyMetadata => object(&[("metadata", GuardrailValue::Object(BTreeMap::new()))]),
    };
    GuardrailCallData {
        litellm_params: Some(litellm_params),
        rest: BTreeMap::new(),
    }
}

fn allow_base(_: &GuardrailCallData, _: GuardrailEventHook) -> bool {
    true
}

fn deny_base(_: &GuardrailCallData, _: GuardrailEventHook) -> bool {
    false
}

fn panic_base(_: &GuardrailCallData, _: GuardrailEventHook) -> bool {
    panic!("the super implementation must not run for a recursive judge call")
}

#[rstest]
#[case::judge_marker_is_suppressed(Data::Marker, true)]
#[case::missing_litellm_params(Data::MissingParams, false)]
#[case::missing_metadata(Data::MissingMetadata, false)]
#[case::non_object_litellm_params(Data::NonObjectParams, false)]
#[case::non_object_metadata(Data::NonObjectMetadata, false)]
#[case::another_origin(Data::WrongOrigin, false)]
#[case::empty_metadata(Data::EmptyMetadata, false)]
fn judge_recursion_tests_behavior(#[case] data: Data, #[case] suppressed: bool) {
    let call = call_data(data);
    assert_eq!(
        is_logged_judge_call(&call, GuardrailEventHook::LoggingOnly),
        suppressed
    );
    assert!(!is_logged_judge_call(&call, GuardrailEventHook::PreCall));
    assert_eq!(
        should_run_guardrail(&call, GuardrailEventHook::LoggingOnly, allow_base),
        !suppressed
    );
    assert!(!should_run_guardrail(
        &call,
        GuardrailEventHook::PreCall,
        deny_base
    ));
    if suppressed {
        assert!(!should_run_guardrail(
            &call,
            GuardrailEventHook::LoggingOnly,
            panic_base
        ));
    }
}
