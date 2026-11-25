#![cfg(test)]

use betterlite_models::decisions::{DecisionsJson, ir};
use betterlite_openai::decisions::call::error::PrepareError;
use betterlite_openai::decisions::call::fields::RequestFormat;

use super::prepare_systemone_call;
use crate::decisions::DecisionsEntryError;
use crate::decisions::decisions_entrypoint::test_support::{
    Fixture, json, prepare, systemone_args, text,
};

#[test]
fn prepare_builds_the_systemone_fields_without_input() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let call = prepare_systemone_call(
        prepare(&resolution, &fixture.logging),
        systemone_args("typesafe"),
    )
    .expect("the call prepares");
    assert!(matches!(call.request, RequestFormat::SystemOne(_)));
    assert_eq!(
        call.ir_request.input,
        ir::DecisionInput::State(DecisionsJson::Text(String::from("state")))
    );
    assert!(call.litellm_params.is_empty());
}

#[test]
fn prepare_rejects_an_input_keyword() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = systemone_args("typesafe");
    args.kwargs
        .insert(String::from("input"), text("go to decisions"));
    let error = prepare_systemone_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("systemone has no input keyword");
    assert!(matches!(
        error,
        DecisionsEntryError::Prepare(PrepareError::OtherFormat { .. })
    ));
    assert_eq!(error.class().metadata().status_code, 400);
    assert_eq!(
        error.to_string(),
        "Requests with 'input' go through litellm.decisions"
    );
}

#[test]
fn prepare_rejects_an_input_keyword_even_when_empty() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = systemone_args("typesafe");
    args.kwargs.insert(String::from("input"), json("null"));
    let error = prepare_systemone_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("an explicit null input still counts as given");
    assert!(matches!(
        error,
        DecisionsEntryError::Prepare(PrepareError::OtherFormat { .. })
    ));
}

#[test]
fn prepare_rejects_questions_that_are_not_a_map() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = systemone_args("typesafe");
    args.questions = Some(json(r#"[{"type":"noul"}]"#));
    let error = prepare_systemone_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("a list is not a System One question map");
    assert!(matches!(error, DecisionsEntryError::Fields { .. }));
    assert_eq!(error.class().metadata().status_code, 400);
    assert!(error.to_string().contains("questions"));
}

#[test]
fn prepare_requires_a_state() {
    let fixture = Fixture::new();
    let resolution = fixture.environment.context();
    let mut args = systemone_args("typesafe");
    args.state = None;
    let error = prepare_systemone_call(prepare(&resolution, &fixture.logging), args)
        .expect_err("the System One envelope requires a state");
    assert!(matches!(
        error,
        DecisionsEntryError::Prepare(PrepareError::InvalidRequest { .. })
    ));
    assert_eq!(error.class().metadata().status_code, 400);
}
