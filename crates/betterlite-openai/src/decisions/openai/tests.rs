#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::error::Category;
use serde_json::{Number, Value};

use betterlite_models::decisions::ir;
use betterlite_models::decisions::openai::{self, ChoiceValue, MessageContent};
use betterlite_models::decisions::systemone::NoulLabel;
use betterlite_models::decisions::{DecisionsJson, Extra, Json, TokenCount};

use super::config::{Credential, OpenAiDecisionsConfig};
use super::request::{SINGLE_OPTION_REASON, ir_to_openai_request, openai_request_to_ir};
use super::response::ResponseConversionError;
use super::response::to_ir::openai_response_to_ir;
use super::response::to_openai::ir_to_openai_response;
use crate::credentials::LiteLlmGlobals;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use crate::decisions::transformation::wire::ProviderRequestWire;

#[rstest]
#[case::choice_question_roundtrip(choice_question_roundtrip)]
#[case::score_question_roundtrip(score_question_roundtrip)]
#[case::predicate_instructions(predicate_instructions)]
#[case::single_option(single_option)]
#[case::missing_text_field(missing_text_field)]
#[case::decision_error(decision_error)]
#[case::decision_answer_alignment(decision_answer_alignment)]
#[case::decision_probability_order(decision_probability_order)]
#[case::decision_usage_model(decision_usage_model)]
#[case::decision_settings(decision_settings)]
fn decisions_cases(#[case] scenario: fn()) {
    scenario();
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn number(value: u8) -> Number {
    Number::from(value)
}

fn decimal(value: f64) -> Number {
    Number::from_f64(value).unwrap()
}

fn text(value: &str) -> DecisionsJson {
    DecisionsJson::Text(value.to_owned())
}

fn state(text_value: &str) -> ir::DecisionInput {
    ir::DecisionInput::State(text(text_value))
}

fn request_with(questions: Vec<ir::DecisionQuestion>) -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: state("the input"),
        questions,
        safety_identifier: None,
    }
}

fn choice_option(value: ChoiceValue, description: Option<DecisionsJson>) -> ir::ChoiceOption {
    ir::ChoiceOption { value, description }
}

fn choice(text_option: &str) -> ChoiceValue {
    ChoiceValue::Text(text_option.to_owned())
}

fn level(label: DecisionsJson, description: Option<&str>) -> ir::ScoreLevel {
    ir::ScoreLevel {
        label,
        description: description.map(str::to_owned),
    }
}

macro_rules! question_fn {
    ($fn_name:ident, $Variant:ident, $Struct:ident, $field:ident, $ValueType:ty) => {
        fn $fn_name(
            name: Option<&str>,
            instructions: Option<DecisionsJson>,
            value: $ValueType,
        ) -> ir::DecisionQuestion {
            ir::DecisionQuestion::$Variant(ir::$Struct {
                name: name.map(str::to_owned),
                instructions,
                $field: value,
                extra: Extra::default(),
            })
        }
    };
}

question_fn!(
    choice_question,
    Choice,
    ChoiceQuestion,
    choices,
    Vec<ir::ChoiceOption>
);
question_fn!(
    score_question,
    Score,
    ScoreQuestion,
    levels,
    Vec<ir::ScoreLevel>
);
question_fn!(
    predicate_question,
    Predicate,
    PredicateQuestion,
    criteria,
    Option<IndexMap<NoulLabel, Option<DecisionsJson>>>
);

fn two_plain_choices() -> Vec<ir::ChoiceOption> {
    vec![
        choice_option(choice("a"), None),
        choice_option(choice("b"), None),
    ]
}

fn wire_json(request: &ir::DecisionRequest, model: &str) -> Value {
    serde_json::to_value(ir_to_openai_request(model, request).unwrap()).unwrap()
}

fn choice_question_roundtrip() {
    ordered_choices_reach_the_wire();
    absent_choice_fields_are_omitted();
    the_openai_body_converts_to_ir();
}

fn roundtrip_request() -> ir::DecisionRequest {
    let mut request = request_with(vec![
        choice_question(
            Some("pick"),
            None,
            vec![
                choice_option(choice("a"), Some(text("first"))),
                choice_option(choice("b"), None),
                choice_option(
                    ChoiceValue::Flag(true),
                    Some(DecisionsJson::Object(IndexMap::from([(
                        "k".to_owned(),
                        Json::Number(number(1)),
                    )]))),
                ),
            ],
        ),
        choice_question(Some("empty"), Some(text("")), two_plain_choices()),
        choice_question(
            None,
            None,
            vec![
                choice_option(ChoiceValue::Flag(false), None),
                choice_option(ChoiceValue::Flag(true), Some(text("affirmative"))),
            ],
        ),
    ]);
    request.safety_identifier = Some("user-1".to_owned());
    request
}

fn ordered_choices_reach_the_wire() {
    let payload = wire_json(&roundtrip_request(), "gpt-4o");
    assert_eq!(payload["model"], "gpt-4o");
    assert_eq!(payload["input"], "the input");
    assert_eq!(payload["safety_identifier"], "user-1");
    assert_eq!(
        payload["questions"][0],
        value(
            r#"{"type": "choice", "name": "pick",
                "instructions": "Which choice best fits the input?",
                "choices": [
                    {"value": "a", "description": "first"},
                    {"value": "b"},
                    {"value": true, "description": "{\"k\": 1}"}
                ]}"#
        )
    );
}

fn absent_choice_fields_are_omitted() {
    let payload = wire_json(&roundtrip_request(), "gpt-4o");
    // an empty instruction string stays empty instead of taking the default
    assert_eq!(payload["questions"][1]["instructions"], "");
    // a question without a name leaves the key out
    assert!(payload["questions"][2].get("name").is_none());
    assert_eq!(
        payload["questions"][2]["choices"][0],
        value(r#"{"value": false}"#)
    );
    assert_eq!(
        payload["questions"][2]["choices"][1],
        value(r#"{"value": true, "description": "affirmative"}"#)
    );
}

fn the_openai_body_converts_to_ir() {
    let body: openai::DecisionRequestBody = serde_json::from_value(value(
        r#"{"input": "the input", "safety_identifier": "user-1",
            "questions": [{"type": "choice", "name": "pick", "instructions": "choose",
                "choices": [
                    {"value": "a", "description": "first"},
                    {"value": "b"},
                    {"value": true}
                ]}]}"#,
    ))
    .unwrap();
    let converted = openai_request_to_ir(&body);
    assert_eq!(converted.safety_identifier, Some("user-1".to_owned()));
    assert_eq!(converted.input, state("the input"));
    let ir::DecisionQuestion::Choice(question) = &converted.questions[0] else {
        panic!("expected a choice question")
    };
    assert_eq!(question.name, Some("pick".to_owned()));
    assert_eq!(question.instructions, Some(text("choose")));
    assert_eq!(
        question.choices,
        vec![
            choice_option(choice("a"), Some(text("first"))),
            choice_option(choice("b"), None),
            choice_option(ChoiceValue::Flag(true), None),
        ]
    );
    assert_eq!(question.extra, Extra::default());
}

fn score_question_roundtrip() {
    score_levels_reach_the_wire();
    the_openai_score_body_converts_to_ir();
    the_ir_score_labels_take_the_question_level();
}

fn score_levels_reach_the_wire() {
    let request = request_with(vec![
        score_question(
            Some("rate"),
            None,
            vec![
                level(text("low"), None),
                level(
                    DecisionsJson::List(vec![Json::Number(number(2))]),
                    Some("desc"),
                ),
            ],
        ),
        score_question(
            None,
            Some(text("")),
            vec![level(text("a"), None), level(text("b"), None)],
        ),
    ]);
    let payload = wire_json(&request, "gpt-4o");
    assert_eq!(
        payload["questions"][0],
        value(
            r#"{"type": "score", "name": "rate",
                "instructions": "Which level best fits the input?",
                "levels": [{"label": "low"}, {"label": "[2]", "description": "desc"}]}"#
        )
    );
    assert_eq!(payload["questions"][1]["instructions"], "");
    assert!(payload["questions"][1].get("name").is_none());
}

fn the_openai_score_body_converts_to_ir() {
    let body: openai::DecisionRequestBody = serde_json::from_value(value(
        r#"{"input": "the input",
            "questions": [{"type": "score", "name": "rate", "instructions": "rate it",
                "levels": [{"label": "low", "description": "bottom"}, {"label": "high"}]}]}"#,
    ))
    .unwrap();
    let converted = openai_request_to_ir(&body);
    let ir::DecisionQuestion::Score(question) = &converted.questions[0] else {
        panic!("expected a score question")
    };
    assert_eq!(question.instructions, Some(text("rate it")));
    assert_eq!(
        question.levels,
        vec![
            level(text("low"), Some("bottom")),
            level(text("high"), None)
        ]
    );
}

fn the_ir_score_labels_take_the_question_level() {
    let request = request_with(vec![score_question(
        Some("rate"),
        None,
        vec![level(text("low"), None), level(text("desc"), None)],
    )]);
    let response: openai::DecisionResponse = serde_json::from_value(value(
        r#"{"model": "gpt-4o",
            "answers": [{"type": "score", "name": "rate", "score": 1.0, "confidence": 0.8,
                "probabilities": [
                    {"value": 1, "label": "returned", "probability": 0.3},
                    {"value": 9, "label": "out-of-range", "probability": 0.4},
                    {"value": -1, "label": "negative", "probability": 0.3}
                ]}],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}"#,
    ))
    .unwrap();
    let converted = openai_response_to_ir(&response, &request);
    let ir::DecisionAnswer::Score { probabilities, .. } = &converted.answers[0] else {
        panic!("expected a score answer")
    };
    // an index inside the levels takes the question's label, anything else the returned one
    assert_eq!(probabilities[0].label, text("desc"));
    assert_eq!(probabilities[1].label, text("out-of-range"));
    assert_eq!(probabilities[2].label, text("negative"));
}

fn predicate_instructions() {
    joined_instructions_and_criteria();
    empty_predicate_text_takes_the_default();
}

fn joined_instructions_and_criteria() {
    let mut criteria: IndexMap<NoulLabel, Option<DecisionsJson>> = IndexMap::new();
    criteria.insert(NoulLabel::True, Some(text("yes")));
    criteria.insert(NoulLabel::False, None);
    let request = request_with(vec![predicate_question(
        Some("truth"),
        Some(DecisionsJson::Object(IndexMap::from([(
            "k".to_owned(),
            Json::List(vec![
                Json::Bool(true),
                Json::Null,
                Json::Text("\u{e9}".to_owned()),
            ]),
        )]))),
        Some(criteria),
    )]);
    let payload = wire_json(&request, "gpt-4o");
    assert_eq!(
        payload["questions"][0]["instructions"],
        "{\"k\": [true, null, \"\\u00e9\"]}\n\nAnswer true when: yes"
    );
}

fn empty_predicate_text_takes_the_default() {
    let default = "Is this true of the input?";
    let no_parts = wire_json(
        &request_with(vec![predicate_question(None, None, None)]),
        "gpt-4o",
    );
    assert_eq!(no_parts["questions"][0]["instructions"], default);

    let empty_instructions = wire_json(
        &request_with(vec![predicate_question(None, Some(text("")), None)]),
        "gpt-4o",
    );
    assert_eq!(empty_instructions["questions"][0]["instructions"], default);

    let mut no_rule: IndexMap<NoulLabel, Option<DecisionsJson>> = IndexMap::new();
    no_rule.insert(NoulLabel::True, None);
    let empty_criteria = wire_json(
        &request_with(vec![predicate_question(None, None, Some(no_rule))]),
        "gpt-4o",
    );
    assert_eq!(empty_criteria["questions"][0]["instructions"], default);
}

fn single_option() {
    one_option_is_unsupported();
    zero_options_are_unsupported();
    predicates_do_not_trigger_the_rule();
}

fn one_option_is_unsupported() {
    let choice_request = request_with(vec![choice_question(
        None,
        None,
        vec![choice_option(choice("only"), None)],
    )]);
    let choice_error = ir_to_openai_request("gpt-4o", &choice_request).unwrap_err();
    assert_eq!(choice_error.to_string(), SINGLE_OPTION_REASON);
    let score_request = request_with(vec![score_question(
        None,
        None,
        vec![level(text("only"), None)],
    )]);
    let score_error = ir_to_openai_request("gpt-4o", &score_request).unwrap_err();
    assert_eq!(score_error.to_string(), SINGLE_OPTION_REASON);
}

fn zero_options_are_unsupported() {
    let choice_request = request_with(vec![choice_question(None, None, Vec::new())]);
    let choice_error = ir_to_openai_request("gpt-4o", &choice_request).unwrap_err();
    assert_eq!(choice_error.to_string(), SINGLE_OPTION_REASON);
    let score_request = request_with(vec![score_question(None, None, Vec::new())]);
    let score_error = ir_to_openai_request("gpt-4o", &score_request).unwrap_err();
    assert_eq!(score_error.to_string(), SINGLE_OPTION_REASON);
}

fn predicates_do_not_trigger_the_rule() {
    let request = request_with(vec![
        predicate_question(None, Some(text("true?")), None),
        choice_question(None, None, two_plain_choices()),
    ]);
    ir_to_openai_request("gpt-4o", &request).unwrap();
}

fn missing_text_field() {
    a_missing_instruction_is_refused();
    an_absent_description_is_left_out();
}

fn a_missing_instruction_is_refused() {
    let error = serde_json::from_value::<openai::DecisionRequestBody>(value(
        r#"{"input": "the input",
            "questions": [{"type": "choice", "choices": [{"value": "a"}, {"value": "b"}]}]}"#,
    ))
    .unwrap_err();
    assert!(error.to_string().contains("instructions"));
    assert_eq!(error.classify(), Category::Data);
}

fn an_absent_description_is_left_out() {
    let request = request_with(vec![
        choice_question(None, None, two_plain_choices()),
        score_question(
            None,
            None,
            vec![level(text("low"), None), level(text("high"), None)],
        ),
    ]);
    let payload = wire_json(&request, "gpt-4o");
    assert!(
        payload["questions"][0]["choices"][0]
            .get("description")
            .is_none()
    );
    assert!(
        payload["questions"][1]["levels"][0]
            .get("description")
            .is_none()
    );
}

fn decision_error() {
    let config = OpenAiDecisionsConfig::default();
    let request = request_with(vec![predicate_question(None, Some(text("true?")), None)]);
    let data_error = config
        .parse_response(
            br#"{"model":"gpt-4o","answers":"not a list","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}"#,
            &request,
        )
        .unwrap_err();
    assert_eq!(data_error.classify(), Category::Data);

    let syntax_error = config.parse_response(b"not json", &request).unwrap_err();
    assert_eq!(syntax_error.classify(), Category::Syntax);

    let valid = config
        .parse_response(
            br#"{"model":"gpt-4o","answers":[{"type":"predicate","name":null,"probability":0.5}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#,
            &request,
        )
        .unwrap();
    assert_eq!(valid.model, Some("gpt-4o".to_owned()));
}

fn decision_answer_alignment() {
    response_answers_are_placed_by_question();
    response_length_mismatch_is_an_error();
    mismatched_variants_are_refusals();
}

fn alignment_request() -> ir::DecisionRequest {
    request_with(vec![
        predicate_question(Some("first"), Some(text("true?")), None),
        choice_question(Some("second"), None, two_plain_choices()),
    ])
}

fn response_answers_are_placed_by_question() {
    let request = alignment_request();
    missing_answer_becomes_a_refusal(&request);
    extra_answers_are_ignored(&request);
    mismatched_answers_become_refusals(&request);
}

fn missing_answer_becomes_a_refusal(request: &ir::DecisionRequest) {
    let missing: openai::DecisionResponse = serde_json::from_value(value(
        r#"{"model": "gpt-4o",
            "answers": [{"type": "predicate", "name": "first", "probability": 0.5}],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}"#,
    ))
    .unwrap();
    let converted = openai_response_to_ir(&missing, request);
    assert_eq!(converted.answers.len(), 2);
    assert!(matches!(
        converted.answers[0],
        ir::DecisionAnswer::Predicate { .. }
    ));
    assert!(matches!(converted.answers[1], ir::DecisionAnswer::Refusal));
}

fn extra_answers_are_ignored(request: &ir::DecisionRequest) {
    let extra: openai::DecisionResponse = serde_json::from_value(value(
        r#"{"model": "gpt-4o",
            "answers": [
                {"type": "predicate", "name": "first", "probability": 0.5},
                {"type": "choice", "name": "second", "choice": "a", "probabilities": [], "confidence": 0.5},
                {"type": "refusal", "name": "third"}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}"#,
    ))
    .unwrap();
    assert_eq!(openai_response_to_ir(&extra, request).answers.len(), 2);
}

fn mismatched_answers_become_refusals(request: &ir::DecisionRequest) {
    let mismatched: openai::DecisionResponse = serde_json::from_value(value(
        r#"{"model": "gpt-4o",
            "answers": [
                {"type": "choice", "name": "first", "choice": "a", "probabilities": [], "confidence": 0.5},
                {"type": "predicate", "name": "second", "probability": 0.5}
            ],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}"#,
    ))
    .unwrap();
    let converted = openai_response_to_ir(&mismatched, request);
    assert!(matches!(converted.answers[0], ir::DecisionAnswer::Refusal));
    assert!(matches!(converted.answers[1], ir::DecisionAnswer::Refusal));
}

fn response_length_mismatch_is_an_error() {
    let request = alignment_request();
    let empty = ir::DecisionResponse {
        model: Some("gpt-4o".to_owned()),
        answers: Vec::new(),
        usage: ir::DecisionUsage::default(),
        extra: Extra::default(),
    };
    let short_error = ir_to_openai_response(&empty, &request, "requested").unwrap_err();
    assert!(matches!(
        short_error,
        ResponseConversionError::AnswerCount {
            answers: 0,
            questions: 2
        }
    ));

    let three = ir::DecisionResponse {
        model: Some("gpt-4o".to_owned()),
        answers: vec![
            ir::DecisionAnswer::Refusal,
            ir::DecisionAnswer::Refusal,
            ir::DecisionAnswer::Refusal,
        ],
        usage: ir::DecisionUsage::default(),
        extra: Extra::default(),
    };
    let long_error = ir_to_openai_response(&three, &request, "requested").unwrap_err();
    assert!(matches!(
        long_error,
        ResponseConversionError::AnswerCount {
            answers: 3,
            questions: 2
        }
    ));
}

fn mismatched_variants_are_refusals() {
    let request = alignment_request();
    let response = ir::DecisionResponse {
        model: Some("gpt-4o".to_owned()),
        answers: vec![
            ir::DecisionAnswer::Predicate {
                probability: number(1),
                extra: Extra::default(),
            },
            ir::DecisionAnswer::Score {
                score: number(1),
                confidence: number(1),
                probabilities: Vec::new(),
                extra: Extra::default(),
            },
        ],
        usage: ir::DecisionUsage::default(),
        extra: Extra::default(),
    };
    let converted = ir_to_openai_response(&response, &request, "requested").unwrap();
    assert_eq!(
        serde_json::to_value(&converted.answers).unwrap(),
        value(
            r#"[{"type": "predicate", "name": "first", "probability": 1},
                {"type": "refusal", "name": "second"}]"#
        )
    );
}

macro_rules! ordering_request {
    () => {
        request_with(vec![
            choice_question(
                Some("pick"),
                None,
                vec![
                    choice_option(ChoiceValue::Flag(true), None),
                    choice_option(choice("b"), None),
                    choice_option(choice("c"), None),
                    choice_option(choice("d"), None),
                ],
            ),
            score_question(
                Some("rate"),
                None,
                vec![
                    level(text("low"), None),
                    level(text("mid"), None),
                    level(text("high"), None),
                ],
            ),
        ])
    };
}

macro_rules! ordering_choice_answer {
    () => {
        ir::DecisionAnswer::Choice {
            choice: ChoiceValue::Flag(true),
            confidence: number(1),
            probabilities: vec![
                ir::ChoiceProbability {
                    value: choice("c"),
                    probability: decimal(0.5_f64),
                },
                ir::ChoiceProbability {
                    value: choice("true"),
                    probability: decimal(0.1_f64),
                },
                ir::ChoiceProbability {
                    value: choice("b"),
                    probability: decimal(0.2_f64),
                },
                ir::ChoiceProbability {
                    value: ChoiceValue::Flag(true),
                    probability: decimal(0.9_f64),
                },
            ],
            extra: Extra::default(),
        }
    };
}

macro_rules! ordering_score_answer {
    () => {
        ir::DecisionAnswer::Score {
            score: number(2),
            confidence: number(1),
            probabilities: vec![
                ir::ScoreProbability {
                    value: number(2),
                    label: text("high"),
                    probability: decimal(0.3_f64),
                },
                ir::ScoreProbability {
                    value: number(0),
                    label: text("low"),
                    probability: decimal(0.4_f64),
                },
                ir::ScoreProbability {
                    value: number(0),
                    label: text("low"),
                    probability: decimal(0.5_f64),
                },
                ir::ScoreProbability {
                    value: number(7),
                    label: text("beyond"),
                    probability: decimal(0.9_f64),
                },
            ],
            extra: Extra::default(),
        }
    };
}

macro_rules! ordering_response {
    () => {
        ir::DecisionResponse {
            model: Some("gpt-4o".to_owned()),
            answers: vec![ordering_choice_answer!(), ordering_score_answer!()],
            usage: ir::DecisionUsage::default(),
            extra: Extra::default(),
        }
    };
}

fn decision_probability_order() {
    let request = ordering_request!();
    let response = ordering_response!();
    let converted = ir_to_openai_response(&response, &request, "requested").unwrap();
    let payload = serde_json::to_value(&converted).unwrap();
    // option order, the last duplicate wins, and a text "true" answers the boolean option
    assert_eq!(
        payload["answers"][0]["probabilities"],
        value(
            r#"[{"value": true, "probability": 0.9},
                {"value": "b", "probability": 0.2},
                {"value": "c", "probability": 0.5},
                {"value": "d", "probability": 0.0}]"#
        )
    );
    // level index order, the last duplicate wins, and a missing level answers 0.0
    assert_eq!(
        payload["answers"][1]["probabilities"],
        value(
            r#"[{"value": 0, "label": "low", "probability": 0.5},
                {"value": 1, "label": "mid", "probability": 0.0},
                {"value": 2, "label": "high", "probability": 0.3}]"#
        )
    );
}

fn decision_usage_model() {
    openai_usage_becomes_ir_usage();
    ir_usage_becomes_openai_usage();
    inputs_keep_their_shape();
    message_sequences_keep_their_order();
}

fn usage() -> ir::DecisionUsage {
    ir::DecisionUsage {
        input_tokens: TokenCount(number(7)),
        output_tokens: TokenCount(number(5)),
        cached_tokens: TokenCount(number(2)),
        cache_write_tokens: TokenCount(number(3)),
        reasoning_tokens: TokenCount(number(4)),
        extra: Extra::default(),
    }
}

fn openai_usage_becomes_ir_usage() {
    let request = request_with(vec![predicate_question(None, Some(text("true?")), None)]);
    let response: openai::DecisionResponse = serde_json::from_value(value(
        r#"{"model": "gpt-4o",
            "answers": [{"type": "predicate", "name": null, "probability": 0.5}],
            "usage": {"input_tokens": 7,
                "input_tokens_details": {"cached_tokens": 2, "cache_write_tokens": 3},
                "output_tokens": 5,
                "output_tokens_details": {"reasoning_tokens": 4},
                "total_tokens": 12}}"#,
    ))
    .unwrap();
    assert_eq!(openai_response_to_ir(&response, &request).usage, usage());
}

fn ir_usage_becomes_openai_usage() {
    let request = request_with(vec![predicate_question(None, Some(text("true?")), None)]);
    let response = ir::DecisionResponse {
        model: None,
        answers: vec![ir::DecisionAnswer::Refusal],
        usage: usage(),
        extra: Extra::default(),
    };
    let rebuilt = ir_to_openai_response(&response, &request, "requested-model").unwrap();
    let payload = serde_json::to_value(&rebuilt).unwrap();
    // a response without its own model names the requested one
    assert_eq!(payload["model"], "requested-model");
    assert_eq!(payload["usage"]["total_tokens"], value("12"));
    assert_eq!(
        payload["usage"]["input_tokens_details"]["cached_tokens"],
        value("2")
    );
    assert_eq!(
        payload["usage"]["input_tokens_details"]["cache_write_tokens"],
        value("3")
    );
    assert_eq!(
        payload["usage"]["output_tokens_details"]["reasoning_tokens"],
        value("4")
    );

    let named = ir::DecisionResponse {
        model: Some("provider-model".to_owned()),
        ..response
    };
    let named_response = ir_to_openai_response(&named, &request, "requested-model").unwrap();
    assert_eq!(named_response.model, "provider-model");
}

fn inputs_keep_their_shape() {
    // a string input becomes the state text and a state object becomes the string'd input
    let string_body: openai::DecisionRequestBody = serde_json::from_value(value(
        r#"{"input": "the input",
            "questions": [{"type": "predicate", "instructions": "true?"}]}"#,
    ))
    .unwrap();
    assert_eq!(openai_request_to_ir(&string_body).input, state("the input"));

    let mut object_state = request_with(vec![predicate_question(None, Some(text("true?")), None)]);
    object_state.input = ir::DecisionInput::State(DecisionsJson::Object(IndexMap::from([(
        "k".to_owned(),
        Json::Number(number(1)),
    )])));
    let payload = wire_json(&object_state, "gpt-4o");
    assert_eq!(payload["input"], "{\"k\": 1}");
}

fn message_sequences_keep_their_order() {
    let messages_body: openai::DecisionRequestBody = serde_json::from_value(value(
        r#"{"input": [
                {"role": "user", "type": "message", "content": "first"},
                {"role": "user", "type": "message", "content": [
                    {"type": "input_text", "text": "second"},
                    {"type": "input_image", "image_url": "https://example.test/i.png"}
                ]}
            ],
            "questions": [{"type": "predicate", "instructions": "true?"}]}"#,
    ))
    .unwrap();
    let converted = openai_request_to_ir(&messages_body);
    let ir::DecisionInput::Messages(messages) = &converted.input else {
        panic!("expected messages")
    };
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].content,
        MessageContent::Text("first".to_owned())
    );
    assert_eq!(
        wire_json(&converted, "gpt-4o")["input"],
        value(
            r#"[{"role": "user", "type": "message", "content": "first"},
                {"role": "user", "type": "message", "content": [
                    {"type": "input_text", "text": "second"},
                    {"type": "input_image", "image_url": "https://example.test/i.png"}
                ]}]"#
        )
    );
}

fn decision_settings() {
    provider_defaults();
    explicit_values_win();
    environment_values_answer();
    empty_environment_names_fall_through();
    globals_beat_the_environment();
    defaults_apply();
    the_provider_body_uses_the_openai_wire();
}

fn provider_defaults() {
    let config = OpenAiDecisionsConfig::default();
    assert_eq!(
        config.get_default_api_base(),
        Some("https://api.openai.com".to_owned())
    );
    assert_eq!(config.request_model("gpt-4o"), "gpt-4o");
    let path: &str = OpenAiDecisionsConfig::PATH;
    assert_eq!(path, "/v1/decisions");
    let supports: bool = OpenAiDecisionsConfig::SUPPORTS_SAFETY_IDENTIFIER;
    assert!(supports);
}

fn lookup(name: &str) -> String {
    match name {
        "OPENAI_BASE_URL" => "env-base".to_owned(),
        "OPENAI_API_BASE" => "legacy-base".to_owned(),
        "OPENAI_API_KEY" => "env-key".to_owned(),
        _ => String::new(),
    }
}

fn explicit_values_win() {
    let config = OpenAiDecisionsConfig::default();
    let base = config.resolve(Some("explicit"), Credential::Base, &lookup);
    assert_eq!(base, Some("explicit".to_owned()));
    let key = config.resolve(Some("explicit-key"), Credential::Key, &lookup);
    assert_eq!(key, Some("explicit-key".to_owned()));
}

fn environment_values_answer() {
    let config = OpenAiDecisionsConfig::default();
    assert_eq!(
        config.resolve(None, Credential::Base, &lookup),
        Some("env-base".to_owned())
    );
    assert_eq!(
        config.resolve(None, Credential::Key, &lookup),
        Some("env-key".to_owned())
    );
    assert_eq!(
        config.resolve_api_base(None, lookup),
        Some("env-base".to_owned())
    );
    assert_eq!(
        config.resolve_api_key(None, lookup),
        Some("env-key".to_owned())
    );
}

fn empty_environment_names_fall_through() {
    let config = OpenAiDecisionsConfig::default();
    // an empty env variable counts as unset, so the next name in the chain answers
    let empty_base_url = |name: &str| match name {
        "OPENAI_API_BASE" => "legacy-base".to_owned(),
        _ => String::new(),
    };
    assert_eq!(
        config.resolve(None, Credential::Base, &empty_base_url),
        Some("legacy-base".to_owned())
    );
}

fn globals_beat_the_environment() {
    let config = OpenAiDecisionsConfig {
        globals: LiteLlmGlobals {
            api_base: Some("litellm-base".to_owned()),
            api_key: Some(SecretString::from("litellm-key")),
            openai_key: Some(SecretString::from("openai-key")),
            organization: None,
        },
    };
    assert_eq!(
        config.resolve(None, Credential::Base, &lookup),
        Some("litellm-base".to_owned())
    );
    assert_eq!(
        config.resolve(None, Credential::Key, &lookup),
        Some("litellm-key".to_owned())
    );
    let only_openai_key = OpenAiDecisionsConfig {
        globals: LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: Some(SecretString::from("openai-key")),
            organization: None,
        },
    };
    assert_eq!(
        only_openai_key.resolve(None, Credential::Key, &lookup),
        Some("openai-key".to_owned())
    );
}

fn defaults_apply() {
    let config = OpenAiDecisionsConfig::default();
    let empty = |_: &str| String::new();
    assert_eq!(
        config.resolve(None, Credential::Base, &empty),
        Some("https://api.openai.com".to_owned())
    );
    assert_eq!(config.resolve(None, Credential::Key, &empty), None);
}

fn the_provider_body_uses_the_openai_wire() {
    let config = OpenAiDecisionsConfig::default();
    let request = request_with(vec![predicate_question(None, Some(text("true?")), None)]);
    let built = config
        .transform_decisions_request("gpt-4o", &request, "openai")
        .unwrap();
    let ProviderRequestWire::OpenAi(wire) = built else {
        panic!("expected an OpenAI wire body")
    };
    assert_eq!(wire.model, "gpt-4o");
}
