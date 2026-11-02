#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use serde_json::error::Category;

use betterlite_models::decisions::openai::{
    ChoiceValue, ContentPart, InputMessage, MessageContent, MessageKind, Role,
};
use betterlite_models::decisions::systemone::{self, NoulLabel, QuestionName};
use betterlite_models::decisions::validators::{Checked, Unchecked};
use betterlite_models::decisions::{DecisionsJson, Extra, Json, TokenCount, ir};

use super::provider_config::{DecisionsProviderConfig, ParseResponseError};
use super::questions::{ir_question, systemone_keys, systemone_question};
use super::requests::{RequestConversionError, ir_to_systemone_request, systemone_request_to_ir};
use super::responses::{
    ir_to_systemone_response, parse_systemone_response, systemone_response_to_ir,
};
use crate::openai_base_config::Headers;

struct BareProviderConfig;

impl DecisionsProviderConfig for BareProviderConfig {}

fn predicate_question(name: &str) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
        name: Some(name.to_owned()),
        instructions: Some(DecisionsJson::Text("is it true?".to_owned())),
        criteria: None,
        extra: Extra::default(),
    })
}

fn unnamed_predicate_question() -> ir::DecisionQuestion {
    ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
        name: None,
        instructions: Some(DecisionsJson::Text("is it true?".to_owned())),
        criteria: None,
        extra: Extra::default(),
    })
}

fn choice_question(name: &str) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Choice(ir::ChoiceQuestion {
        name: Some(name.to_owned()),
        instructions: Some(DecisionsJson::Text("pick one".to_owned())),
        choices: vec![
            ir::ChoiceOption {
                value: ChoiceValue::Text("yes".to_owned()),
                description: Some(DecisionsJson::Text("affirmative".to_owned())),
            },
            ir::ChoiceOption {
                value: ChoiceValue::Text("no".to_owned()),
                description: Some(DecisionsJson::Text("negative".to_owned())),
            },
        ],
        extra: Extra::default(),
    })
}

fn score_question(name: &str) -> ir::DecisionQuestion {
    ir::DecisionQuestion::Score(ir::ScoreQuestion {
        name: Some(name.to_owned()),
        instructions: Some(DecisionsJson::Text("rate it".to_owned())),
        levels: vec![
            ir::ScoreLevel {
                label: DecisionsJson::Text("low".to_owned()),
                description: None,
            },
            ir::ScoreLevel {
                label: DecisionsJson::Text("high".to_owned()),
                description: Some("very high indeed".to_owned()),
            },
        ],
        extra: Extra::default(),
    })
}

fn request_with(questions: Vec<ir::DecisionQuestion>) -> ir::DecisionRequest {
    ir::DecisionRequest {
        input: ir::DecisionInput::State(DecisionsJson::Text("state".to_owned())),
        questions,
        safety_identifier: None,
    }
}

#[rstest]
#[case::all_named(
    vec![predicate_question("a"), choice_question("b"), score_question("c")],
    vec!["a", "b", "c"],
)]
#[case::one_unnamed_falls_back_for_all_questions(
    vec![predicate_question("a"), unnamed_predicate_question(), score_question("c")],
    vec!["0", "1", "2"],
)]
#[case::duplicate_names(
    vec![predicate_question("a"), predicate_question("a")],
    vec!["0", "1"],
)]
fn systemone_keys_falls_back_to_numeric_indices_on_missing_or_duplicate_names(
    #[case] questions: Vec<ir::DecisionQuestion>,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(systemone_keys(&questions), expected);
}

#[test]
fn systemone_keys_returns_empty_for_no_questions() {
    assert!(systemone_keys(&[]).is_empty());
}

#[test]
fn ir_question_maps_noul_to_predicate() {
    let mut criteria = IndexMap::new();
    criteria.insert(
        NoulLabel::True,
        Some(DecisionsJson::Text("good".to_owned())),
    );
    let noul = systemone::NoulQuestion {
        instructions: Some(DecisionsJson::Text("is it true?".to_owned())),
        criteria: Some(criteria.clone()),
        extra: Extra::default(),
    };
    let checked: Checked<systemone::NoulQuestion> = Unchecked(noul).try_into().unwrap();
    let question = ir_question("a".to_owned(), &systemone::DecisionQuestion::Noul(checked));
    match question {
        ir::DecisionQuestion::Predicate(predicate) => {
            assert_eq!(predicate.name, Some("a".to_owned()));
            assert_eq!(predicate.criteria, Some(criteria));
        }
        other @ (ir::DecisionQuestion::Choice(_) | ir::DecisionQuestion::Score(_)) => {
            panic!("expected a predicate question, got {other:?}")
        }
    }
}

#[test]
fn ir_question_maps_choice_criteria_keys_to_text_values() {
    let mut criteria = IndexMap::new();
    criteria.insert(
        "yes".to_owned(),
        Some(DecisionsJson::Text("affirmative".to_owned())),
    );
    criteria.insert("no".to_owned(), None);
    let question = systemone::DecisionQuestion::Choice(systemone::ChoiceQuestion {
        instructions: Some(DecisionsJson::Text("pick one".to_owned())),
        criteria: Unchecked(criteria).try_into().unwrap(),
        extra: Extra::default(),
    });
    let ir_question = ir_question("b".to_owned(), &question);
    match ir_question {
        ir::DecisionQuestion::Choice(choice) => {
            assert_eq!(choice.name, Some("b".to_owned()));
            assert_eq!(
                choice.choices,
                vec![
                    ir::ChoiceOption {
                        value: ChoiceValue::Text("yes".to_owned()),
                        description: Some(DecisionsJson::Text("affirmative".to_owned())),
                    },
                    ir::ChoiceOption {
                        value: ChoiceValue::Text("no".to_owned()),
                        description: None,
                    },
                ],
            );
        }
        other @ (ir::DecisionQuestion::Predicate(_) | ir::DecisionQuestion::Score(_)) => {
            panic!("expected a choice question, got {other:?}")
        }
    }
}

#[test]
fn ir_question_maps_score_criteria_to_level_labels() {
    let criteria = vec![
        DecisionsJson::Text("low".to_owned()),
        DecisionsJson::Text("high".to_owned()),
    ];
    let question = systemone::DecisionQuestion::Score(systemone::ScoreQuestion {
        instructions: Some(DecisionsJson::Text("rate it".to_owned())),
        criteria: Unchecked(criteria).try_into().unwrap(),
        extra: Extra::default(),
    });
    let ir_question = ir_question("c".to_owned(), &question);
    match ir_question {
        ir::DecisionQuestion::Score(score) => {
            assert_eq!(score.name, Some("c".to_owned()));
            assert_eq!(
                score.levels,
                vec![
                    ir::ScoreLevel {
                        label: DecisionsJson::Text("low".to_owned()),
                        description: None,
                    },
                    ir::ScoreLevel {
                        label: DecisionsJson::Text("high".to_owned()),
                        description: None,
                    },
                ],
            );
        }
        other @ (ir::DecisionQuestion::Predicate(_) | ir::DecisionQuestion::Choice(_)) => {
            panic!("expected a score question, got {other:?}")
        }
    }
}

fn sample_systemone_request() -> systemone::DecisionsRequest {
    let mut questions = IndexMap::new();
    let noul = systemone::NoulQuestion {
        instructions: Some(DecisionsJson::Text("is it true?".to_owned())),
        criteria: None,
        extra: Extra::default(),
    };
    let checked: Checked<systemone::NoulQuestion> = Unchecked(noul).try_into().unwrap();
    let name: QuestionName = Unchecked("a".to_owned()).try_into().unwrap();
    questions.insert(name, systemone::DecisionQuestion::Noul(checked));
    systemone::DecisionsRequest {
        model: "systemone-1".to_owned(),
        body: systemone::DecisionsRequestBody {
            state: DecisionsJson::Text("hello".to_owned()),
            questions: Unchecked(questions).try_into().unwrap(),
            extra: Extra::default(),
        },
    }
}

#[test]
fn systemone_request_to_ir_keys_questions_by_name() {
    let request = sample_systemone_request();
    let ir = systemone_request_to_ir(&request.body);
    assert_eq!(
        ir.input,
        ir::DecisionInput::State(DecisionsJson::Text("hello".to_owned()))
    );
    assert_eq!(ir.questions.len(), 1);
    match &ir.questions[0] {
        ir::DecisionQuestion::Predicate(predicate) => {
            assert_eq!(predicate.name, Some("a".to_owned()));
        }
        other @ (ir::DecisionQuestion::Choice(_) | ir::DecisionQuestion::Score(_)) => {
            panic!("expected a predicate question, got {other:?}")
        }
    }
}

#[test]
fn systemone_request_builds_model_state_and_questions() {
    let request = request_with(vec![predicate_question("a"), choice_question("b")]);
    let built = ir_to_systemone_request("systemone-1", &request).unwrap();
    assert_eq!(built.model, "systemone-1");
    assert_eq!(built.body.state, DecisionsJson::Text("state".to_owned()));
    let names: Vec<&str> = built
        .body
        .questions
        .as_ref()
        .keys()
        .map(|name| name.as_ref().as_str())
        .collect();
    assert_eq!(names, vec!["a", "b"]);
}

#[test]
fn systemone_request_rejects_image_content_on_openai_input() {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::Messages(vec![InputMessage {
            role: Role::default(),
            kind: MessageKind::default(),
            content: MessageContent::Parts(vec![ContentPart::InputImage {
                image_url: "https://example.com/a.png".to_owned(),
                detail: None,
            }]),
        }]),
        questions: vec![predicate_question("a")],
        safety_identifier: None,
    };
    let error = ir_to_systemone_request("systemone-1", &request).unwrap_err();
    assert!(matches!(error, RequestConversionError::Unsupported(_)));
}

#[test]
fn systemone_request_concatenates_text_message_parts_into_state() {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::Messages(vec![InputMessage {
            role: Role::default(),
            kind: MessageKind::default(),
            content: MessageContent::Parts(vec![
                ContentPart::InputText {
                    text: "first".to_owned(),
                },
                ContentPart::InputText {
                    text: "second".to_owned(),
                },
            ]),
        }]),
        questions: vec![predicate_question("a")],
        safety_identifier: None,
    };
    let built = ir_to_systemone_request("systemone-1", &request).unwrap();
    assert_eq!(
        built.body.state,
        DecisionsJson::Text("first\n\nsecond".to_owned())
    );
}

fn sample_ir_response() -> (ir::DecisionRequest, ir::DecisionResponse) {
    let request = request_with(vec![
        predicate_question("a"),
        choice_question("b"),
        score_question("c"),
    ]);
    let response = ir::DecisionResponse {
        model: Some("systemone-1".to_owned()),
        answers: vec![
            ir::DecisionAnswer::Predicate {
                probability: Number::from_f64(0.9).unwrap(),
                extra: Extra::default(),
            },
            ir::DecisionAnswer::Choice {
                choice: ChoiceValue::Text("yes".to_owned()),
                confidence: Number::from_f64(0.8).unwrap(),
                probabilities: vec![
                    ir::ChoiceProbability {
                        value: ChoiceValue::Text("yes".to_owned()),
                        probability: Number::from_f64(0.8).unwrap(),
                    },
                    ir::ChoiceProbability {
                        value: ChoiceValue::Text("no".to_owned()),
                        probability: Number::from_f64(0.2).unwrap(),
                    },
                ],
                extra: Extra::default(),
            },
            ir::DecisionAnswer::Refusal,
        ],
        usage: ir::DecisionUsage {
            input_tokens: TokenCount(Number::from(10_u32)),
            output_tokens: TokenCount(Number::from(5_u32)),
            cached_tokens: TokenCount(Number::from(0_u32)),
            cache_write_tokens: TokenCount(Number::from(0_u32)),
            reasoning_tokens: TokenCount(Number::from(0_u32)),
            extra: Extra::default(),
        },
        extra: Extra::default(),
    };
    (request, response)
}

#[test]
fn systemone_response_drops_refusals_when_converting_to_systemone() {
    let (request, response) = sample_ir_response();
    let built = ir_to_systemone_response(&response, &request);
    assert_eq!(built.model, Some("systemone-1".to_owned()));
    assert!(!built.answers.contains_key("c"));
    assert_eq!(built.answers.len(), 2);
    match built.answers.get("b").unwrap() {
        systemone::DecisionAnswer::Choice { choice, .. } => assert_eq!(choice, "yes"),
        other @ (systemone::DecisionAnswer::Noul { .. }
        | systemone::DecisionAnswer::Score { .. }) => {
            panic!("expected a choice answer, got {other:?}")
        }
    }
}

#[test]
fn systemone_response_to_ir_matches_answers_by_numeric_key_when_questions_are_unnamed() {
    let request = request_with(vec![
        unnamed_predicate_question(),
        unnamed_predicate_question(),
    ]);
    let mut answers = IndexMap::new();
    answers.insert(
        "0".to_owned(),
        systemone::DecisionAnswer::Noul {
            noul: Number::from_f64(0.1).unwrap(),
            extra: Extra::default(),
        },
    );
    answers.insert(
        "1".to_owned(),
        systemone::DecisionAnswer::Noul {
            noul: Number::from_f64(0.9).unwrap(),
            extra: Extra::default(),
        },
    );
    let response = systemone::DecisionsResponse {
        model: Some("systemone-1".to_owned()),
        answers,
        usage: None,
        hidden_params: Extra::default(),
        extra: Extra::default(),
    };
    let ir = systemone_response_to_ir(&response, &request);
    assert_eq!(ir.answers.len(), 2);
    match &ir.answers[1] {
        ir::DecisionAnswer::Predicate { probability, .. } => {
            assert_eq!(*probability, Number::from_f64(0.9).unwrap());
        }
        other @ (ir::DecisionAnswer::Choice { .. }
        | ir::DecisionAnswer::Score { .. }
        | ir::DecisionAnswer::Refusal) => {
            panic!("expected a predicate answer, got {other:?}")
        }
    }
}

#[test]
fn systemone_response_reconstructs_score_answers_with_level_labels() {
    let request = request_with(vec![score_question("c")]);
    let mut legend = IndexMap::new();
    legend.insert("0".to_owned(), DecisionsJson::Text("low".to_owned()));
    legend.insert("1".to_owned(), DecisionsJson::Text("high".to_owned()));
    let mut score_probabilities = IndexMap::new();
    score_probabilities.insert("0".to_owned(), Number::from_f64(0.1).unwrap());
    score_probabilities.insert("1".to_owned(), Number::from_f64(0.9).unwrap());
    let mut answers = IndexMap::new();
    answers.insert(
        "c".to_owned(),
        systemone::DecisionAnswer::Score {
            score: Number::from(1_u8),
            confidence: Number::from_f64(0.9).unwrap(),
            legend,
            probabilities: score_probabilities,
            extra: Extra::default(),
        },
    );
    let response = systemone::DecisionsResponse {
        model: Some("systemone-1".to_owned()),
        answers,
        usage: None,
        hidden_params: Extra::default(),
        extra: Extra::default(),
    };
    let ir = systemone_response_to_ir(&response, &request);
    match &ir.answers[0] {
        ir::DecisionAnswer::Score { probabilities, .. } => {
            assert_eq!(
                probabilities[1].label,
                DecisionsJson::Text("high".to_owned())
            );
            assert_eq!(probabilities[1].probability, Number::from_f64(0.9).unwrap());
        }
        other @ (ir::DecisionAnswer::Predicate { .. }
        | ir::DecisionAnswer::Choice { .. }
        | ir::DecisionAnswer::Refusal) => {
            panic!("expected a score answer, got {other:?}")
        }
    }
}

#[test]
fn systemone_response_parse_succeeds_on_well_formed_payload() {
    let request = request_with(vec![predicate_question("a")]);
    let payload = br#"{"model":"systemone-1","answers":{"a":{"type":"noul","noul":0.5}}}"#;
    let ir = parse_systemone_response(payload, &request).unwrap();
    assert_eq!(ir.model, Some("systemone-1".to_owned()));
}

#[test]
fn systemone_response_parse_fails_on_malformed_json() {
    let request = request_with(vec![predicate_question("a")]);
    let error = parse_systemone_response(b"not json", &request).unwrap_err();
    assert_eq!(error.classify(), Category::Syntax);
}

#[test]
fn systemone_response_parse_fails_on_schema_mismatch() {
    let request = request_with(vec![predicate_question("a")]);
    let error = parse_systemone_response(br#"{"answers": "not an object"}"#, &request).unwrap_err();
    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn base_config_resolve_api_base_prefers_explicit_value_over_env_and_default() {
    let config = BareProviderConfig;
    let get_secret_str = |_: &str| "from-env".to_owned();
    assert_eq!(
        config.resolve_api_base(Some("explicit"), get_secret_str),
        Some("explicit".to_owned())
    );
}

#[test]
fn base_config_resolve_api_base_falls_back_to_default_when_env_is_unset() {
    struct WithDefault;
    impl DecisionsProviderConfig for WithDefault {
        fn get_default_api_base(&self) -> Option<String> {
            Some("https://default.example".to_owned())
        }
    }
    let config = WithDefault;
    let get_secret_str = |_: &str| String::new();
    assert_eq!(
        config.resolve_api_base(None, get_secret_str),
        Some("https://default.example".to_owned())
    );
}

#[test]
fn base_config_resolve_api_key_returns_first_nonempty_env_name() {
    struct TwoEnvNames;
    impl DecisionsProviderConfig for TwoEnvNames {
        const API_KEY_ENV: &'static [&'static str] = &["FIRST_KEY", "SECOND_KEY"];
    }
    let config = TwoEnvNames;
    let get_secret_str = |name: &str| {
        if name == "SECOND_KEY" {
            "secret".to_owned()
        } else {
            String::new()
        }
    };
    assert_eq!(
        config.resolve_api_key(None, get_secret_str),
        Some("secret".to_owned())
    );
}

#[test]
fn base_config_validate_environment_filters_reserved_headers_case_insensitively() {
    let config = BareProviderConfig;
    let mut headers: Headers = Headers::new();
    headers.insert("Authorization".to_owned(), "stale".to_owned());
    headers.insert("CONTENT-TYPE".to_owned(), "text/plain".to_owned());
    headers.insert("X-Custom".to_owned(), "kept".to_owned());
    let api_key = SecretString::from("secret-value");
    let validated = config.validate_environment(&headers, "model", Some(&api_key));
    assert_eq!(validated.get("X-Custom"), Some(&"kept".to_owned()));
    assert_eq!(
        validated.get("Authorization"),
        Some(&"Bearer secret-value".to_owned())
    );
    assert_eq!(
        validated.get("Content-Type"),
        Some(&"application/json".to_owned())
    );
}

#[test]
fn base_config_validate_environment_omits_authorization_without_an_api_key() {
    let config = BareProviderConfig;
    let validated = config.validate_environment(&Headers::new(), "model", None);
    assert_eq!(validated.get("Authorization"), None);
    assert_eq!(
        validated.get("Content-Type"),
        Some(&"application/json".to_owned())
    );
}

#[rstest]
#[case::trailing_slash("https://api.example.com/", "https://api.example.com/v1/systemone")]
#[case::trailing_v1("https://api.example.com/v1", "https://api.example.com/v1/systemone")]
#[case::trailing_slash_and_v1(
    "https://api.example.com/v1/",
    "https://api.example.com/v1/systemone"
)]
#[case::no_trailing("https://api.example.com", "https://api.example.com/v1/systemone")]
fn base_config_get_complete_url_normalizes_trailing_slash_and_v1_suffix(
    #[case] api_base: &str,
    #[case] expected: &str,
) {
    let config = BareProviderConfig;
    assert_eq!(config.get_complete_url(api_base, "model"), expected);
}

#[test]
fn base_config_signs_with_aws_defaults_to_false() {
    let config = BareProviderConfig;
    assert!(!config.signs_with_aws("https://api.example.com"));
}

#[test]
fn base_config_sign_request_returns_headers_unchanged_and_no_signed_body() {
    use super::provider_config::SigningContext;

    let config = BareProviderConfig;
    let mut headers: Headers = Headers::new();
    headers.insert("X-Custom".to_owned(), "kept".to_owned());
    let body = sample_systemone_request();
    let litellm_params = Extra::default();
    let context = SigningContext {
        url: "https://api.example.com/v1/systemone",
        api_base: "https://api.example.com",
        body: &body,
        api_key: None,
        litellm_params: &litellm_params,
    };
    let (signed_headers, signed_body) = config.sign_request(headers.clone(), context);
    assert_eq!(signed_headers, headers);
    assert!(signed_body.is_none());
}

#[test]
fn base_config_provider_reported_cost_defaults_to_none() {
    let config = BareProviderConfig;
    let (_, response) = sample_ir_response();
    assert_eq!(config.provider_reported_cost(&response), None);
}

#[test]
fn base_config_missing_api_base_message_names_the_provider() {
    let config = BareProviderConfig;
    assert_eq!(
        config.missing_api_base_message("typesafe"),
        "api_base is required for Decisions provider 'typesafe'"
    );
}

#[test]
fn base_config_transform_decisions_request_builds_systemone_request_from_ir() {
    let config = BareProviderConfig;
    let request = request_with(vec![predicate_question("a")]);
    let built = config
        .transform_decisions_request("model-1", &request, "typesafe")
        .unwrap();
    assert_eq!(built.model, "model-1");
}

#[test]
fn base_config_transform_decisions_response_wraps_schema_mismatch_as_invalid() {
    let config = BareProviderConfig;
    let request = request_with(vec![predicate_question("a")]);
    let error = config
        .transform_decisions_response(
            "model-1",
            "typesafe",
            br#"{"answers": "not an object"}"#,
            &request,
        )
        .unwrap_err();
    assert!(matches!(error, ParseResponseError::Invalid(_)));
}

#[test]
fn base_config_transform_decisions_response_surfaces_malformed_json_without_wrapping() {
    let config = BareProviderConfig;
    let request = request_with(vec![predicate_question("a")]);
    let error = config
        .transform_decisions_response("model-1", "typesafe", b"not json", &request)
        .unwrap_err();
    assert!(matches!(error, ParseResponseError::Malformed(_)));
}

#[test]
fn base_config_get_error_class_carries_status_code_and_message() {
    let config = BareProviderConfig;
    let error = config.get_error_class("slow down", 429, &Headers::new());
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
}

#[test]
fn roundtrip_preserves_question_names_choices_scores_and_extra_fields() {
    let mut predicate_extra = Extra::default();
    predicate_extra.insert("note".to_owned(), Json::Text("kept".to_owned()));
    let original = ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
        name: Some("a".to_owned()),
        instructions: Some(DecisionsJson::Text("is it true?".to_owned())),
        criteria: None,
        extra: predicate_extra,
    });
    let roundtripped = ir_question("a".to_owned(), &systemone_question(&original).unwrap());
    assert_eq!(roundtripped, original);

    let original_choice = choice_question("b");
    let systemone_choice = systemone_question(&original_choice).unwrap();
    let roundtripped_choice = ir_question("b".to_owned(), &systemone_choice);
    assert_eq!(roundtripped_choice, original_choice);
}

#[test]
fn roundtrip_preserves_answer_confidences_and_choice_probabilities() {
    let (request, response) = sample_ir_response();
    let systemone_response = ir_to_systemone_response(&response, &request);
    let mut payload = Vec::new();
    serde_json::to_writer(&mut payload, &systemone_response).unwrap();
    let rebuilt = parse_systemone_response(&payload, &request).unwrap();
    match (&rebuilt.answers[1], &response.answers[1]) {
        (
            ir::DecisionAnswer::Choice {
                choice,
                confidence,
                probabilities,
                ..
            },
            ir::DecisionAnswer::Choice {
                choice: original_choice,
                confidence: original_confidence,
                probabilities: original_probabilities,
                ..
            },
        ) => {
            assert_eq!(choice, original_choice);
            assert_eq!(confidence, original_confidence);
            assert_eq!(probabilities, original_probabilities);
        }
        other => panic!("expected choice answers on both sides, got {other:?}"),
    }
}

#[test]
fn roundtrip_openai_input_to_ir_text_is_semantically_equivalent_to_the_source_messages() {
    let request = ir::DecisionRequest {
        input: ir::DecisionInput::Messages(vec![
            InputMessage {
                role: Role::default(),
                kind: MessageKind::default(),
                content: MessageContent::Text("hello".to_owned()),
            },
            InputMessage {
                role: Role::default(),
                kind: MessageKind::default(),
                content: MessageContent::Parts(vec![ContentPart::InputText {
                    text: "world".to_owned(),
                }]),
            },
        ]),
        questions: vec![predicate_question("a")],
        safety_identifier: None,
    };
    let built = ir_to_systemone_request("model-1", &request).unwrap();
    assert_eq!(
        built.body.state,
        DecisionsJson::Text("hello\n\nworld".to_owned())
    );
}
