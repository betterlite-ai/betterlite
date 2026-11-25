#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::decisions::call::fields::RequestFields;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{FieldShapeError, request_fields};

const DEPRECATION_MESSAGE: &str = "litellm.decisions(state=...) is deprecated, call litellm.systemone for the System One request format";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Captured {
    message: String,
    category: String,
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<Captured>>>);

impl Warnings {
    fn captured(&self) -> Vec<Captured> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Visit for Captured {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "category" => self.category = value.to_owned(),
            _other => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut captured = Captured::default();
        event.record(&mut captured);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(captured);
    }
}

fn capture_warnings() -> (Warnings, DefaultGuard) {
    let warnings = Warnings::default();
    let guard = set_default(registry().with(warnings.clone()));
    (warnings, guard)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn json(value: &str) -> Json {
    serde_json::from_str(value).expect("the test JSON parses")
}

fn with_state(state: Json) -> Extra {
    let mut kwargs = Extra::new();
    kwargs.insert(String::from("state"), state);
    kwargs
}

#[test]
fn openai_fields_keep_the_input_and_questions() {
    let questions = json(r#"[{"type":"predicate","name":"first","instructions":"Is it true?"}]"#);
    let fields = request_fields(Some(text("state")), Some(questions.clone()), &Extra::new())
        .expect("the fields build");
    let RequestFields::OpenAi(fields) = fields else {
        panic!("no state keyword means the OpenAI format");
    };
    assert_eq!(fields.input, Some(text("state")));
    assert_eq!(
        fields.questions,
        Some(vec![json(
            r#"{"type":"predicate","name":"first","instructions":"Is it true?"}"#
        ),])
    );
}

#[test]
fn openai_fields_treat_a_null_questions_value_as_absent() {
    let fields = request_fields(None, Some(Json::Null), &Extra::new()).expect("the fields build");
    let RequestFields::OpenAi(fields) = fields else {
        panic!("no state keyword means the OpenAI format");
    };
    assert_eq!(fields.questions, None);
}

#[test]
fn openai_fields_treat_a_missing_questions_value_as_absent() {
    let fields =
        request_fields(Some(text("state")), None, &Extra::new()).expect("the fields build");
    let RequestFields::OpenAi(fields) = fields else {
        panic!("no state keyword means the OpenAI format");
    };
    assert_eq!(fields.input, Some(text("state")));
    assert_eq!(fields.questions, None);
}

#[test]
fn openai_fields_reject_a_questions_map() {
    let error = request_fields(None, Some(json(r#"{"first":{}}"#)), &Extra::new())
        .expect_err("a map is not an OpenAI question list");
    assert_eq!(error, FieldShapeError::QuestionsNotAList);
}

#[test]
fn systemone_fields_warn_and_keep_the_state() {
    let (warnings, _guard) = capture_warnings();
    let questions = json(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#);
    let fields = request_fields(None, Some(questions), &with_state(text("state")))
        .expect("the fields build");
    let RequestFields::SystemOne(fields) = fields else {
        panic!("a state keyword means the System One format");
    };
    assert_eq!(fields.state, Some(text("state")));
    let expected: IndexMap<String, Json> =
        serde_json::from_str(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#)
            .expect("the questions parse");
    assert_eq!(fields.questions, Some(expected));
    assert_eq!(
        warnings.captured(),
        vec![Captured {
            message: String::from(DEPRECATION_MESSAGE),
            category: String::from("DeprecationWarning"),
        }]
    );
}

#[test]
fn systemone_fields_keep_a_null_state_out() {
    let (warnings, _guard) = capture_warnings();
    let fields = request_fields(None, None, &with_state(Json::Null)).expect("the fields build");
    let RequestFields::SystemOne(fields) = fields else {
        panic!("a null state still takes the deprecated path");
    };
    assert_eq!(fields.state, None);
    assert_eq!(fields.questions, None);
    assert_eq!(warnings.captured().len(), 1);
}

#[test]
fn systemone_fields_treat_a_null_questions_map_as_absent() {
    let (warnings, _guard) = capture_warnings();
    let fields = request_fields(None, Some(Json::Null), &with_state(text("state")))
        .expect("the fields build");
    let RequestFields::SystemOne(fields) = fields else {
        panic!("a state keyword means the System One format");
    };
    assert_eq!(fields.state, Some(text("state")));
    assert_eq!(fields.questions, None);
    assert_eq!(warnings.captured().len(), 1);
}

#[test]
fn systemone_fields_reject_a_questions_list() {
    let (warnings, _guard) = capture_warnings();
    let error = request_fields(
        None,
        Some(json(r#"[{"type":"noul"}]"#)),
        &with_state(text("s")),
    )
    .expect_err("a list is not a System One question map");
    assert_eq!(error, FieldShapeError::QuestionsNotAMap);
    assert_eq!(warnings.captured().len(), 1);
}

#[test]
fn systemone_fields_reject_a_boolean_state() {
    let (warnings, _guard) = capture_warnings();
    let error = request_fields(None, None, &with_state(Json::Bool(true)))
        .expect_err("a boolean is not Decision JSON");
    assert_eq!(error, FieldShapeError::StateNotDecisionJson);
    assert_eq!(warnings.captured().len(), 1);
}
