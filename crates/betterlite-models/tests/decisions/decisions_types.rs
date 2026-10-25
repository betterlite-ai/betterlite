use betterlite_models::decisions::openai::{self, ChoiceValue, DecisionRequest, DecisionResponse};
use betterlite_models::decisions::systemone::{
    self, DecisionQuestionMap, DecisionsRequest, DecisionsResponse, DecisionsUsage,
};
use betterlite_models::decisions::{DecisionsJson, Extra, TokenCount, ir};
use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Number;

fn round_trip<Record>(json: &str) -> String
where
    Record: DeserializeOwned + Serialize,
{
    serde_json::to_string(&serde_json::from_str::<Record>(json).unwrap()).unwrap()
}

#[test]
fn keeps_systemone_records_as_sent() {
    let request = r#"{"model":"judge","state":{"z":1,"a":[2.5,null]},"questions":{"urgent":{"type":"noul","instructions":"Urgent?","criteria":{"true":"now","false":null},"weight":2},"team":{"type":"choice","instructions":null,"criteria":{"tech":"bugs","billing":null}},"tone":{"type":"score","instructions":"Rate","criteria":["calm",{"label":"angry"}]}},"trace":"t1"}"#;
    assert_eq!(round_trip::<DecisionsRequest>(request), request);
    let response = r#"{"model":null,"answers":{"urgent":{"type":"noul","noul":0.75},"team":{"type":"choice","choice":"tech","confidence":0.9,"probabilities":{"tech":0.9,"billing":0.1}},"tone":{"type":"score","score":1.0,"confidence":0.5,"legend":{"0":"calm"},"probabilities":{"0":0.5,"1":0.5}}},"usage":null,"id":"r1"}"#;
    assert_eq!(round_trip::<DecisionsResponse>(response), response);
}

#[test]
fn keeps_openai_records_as_sent() {
    let request = r#"{"model":"judge","input":[{"role":"user","type":"message","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"https://example.com/a.png","detail":null}]}],"questions":[{"type":"predicate","name":"spam","instructions":"Spam?"},{"type":"choice","name":null,"instructions":"Lang?","choices":[{"value":true,"description":null},{"value":"no","description":"No"}]},{"type":"score","name":"tone","instructions":"Rate","levels":[{"label":"low","description":null},{"label":"high","description":null}]}],"safety_identifier":null,"store":false}"#;
    assert_eq!(round_trip::<DecisionRequest>(request), request);
    let response = r#"{"model":"judge","answers":[{"type":"predicate","name":"spam","probability":0.25},{"type":"choice","name":null,"choice":true,"probabilities":[{"value":true,"probability":0.75}],"confidence":0.75},{"type":"score","name":"tone","score":1.0,"probabilities":[{"value":1,"label":"high","probability":1.0}],"confidence":1.0},{"type":"refusal","name":null}],"usage":{"input_tokens":5,"input_tokens_details":{"cached_tokens":1,"cache_write_tokens":2},"output_tokens":2,"output_tokens_details":{"reasoning_tokens":1},"total_tokens":7},"id":"r1"}"#;
    assert_eq!(round_trip::<DecisionResponse>(response), response);
}

#[test]
fn fills_defaults_and_hides_cache_tokens() {
    let usage: DecisionsUsage =
        serde_json::from_str(r#"{"input_tokens":3,"cached_tokens":2,"cache_write_tokens":1}"#)
            .unwrap();
    assert_eq!(
        (&usage.cached_tokens, &usage.cache_write_tokens),
        (
            &TokenCount(Number::from(2_u8)),
            &TokenCount(Number::from(1_u8))
        )
    );
    assert_eq!(
        serde_json::to_string(&usage).unwrap(),
        r#"{"input_tokens":3,"output_tokens":0}"#
    );
    let message = round_trip::<openai::InputMessage>(r#"{"content":"hi"}"#);
    assert_eq!(
        message,
        r#"{"role":"user","type":"message","content":"hi"}"#
    );
    let openai_usage = round_trip::<openai::DecisionUsage>(
        r#"{"input_tokens":1,"output_tokens":2,"total_tokens":3}"#,
    );
    let details = r#""input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}"#;
    assert!(openai_usage.contains(details) && openai_usage.contains(r#"{"reasoning_tokens":0}"#));
}

#[test]
fn builds_ir_records() {
    let option = ir::ChoiceOption {
        value: ChoiceValue::Flag(true),
        description: None,
    };
    let question = ir::ChoiceQuestion {
        name: None,
        instructions: None,
        choices: vec![option],
        extra: Extra::new(),
    };
    let state = ir::DecisionInput::State(DecisionsJson::Text("ticket".to_owned()));
    let request = ir::DecisionRequest {
        input: state,
        questions: vec![ir::DecisionQuestion::Choice(question)],
        safety_identifier: None,
    };
    assert_eq!(request.questions.len(), 1);
    let usage = ir::DecisionUsage::default();
    let zero = TokenCount(Number::from(0_u8));
    assert_eq!(
        [usage.input_tokens, usage.reasoning_tokens],
        [zero.clone(), zero]
    );
    let unsupported = ir::UnsupportedRequest {
        reason: "text only".to_owned(),
    };
    assert_eq!(unsupported.to_string(), "text only");
}

#[derive(Clone, Copy)]
enum Bound {
    Questions,
    ChoiceCriteria,
    ScoreCriteria,
    Choices,
    Levels,
    OpenaiQuestions,
}

fn joined(count: usize, template: &str) -> String {
    let items: Vec<String> = (0..count)
        .map(|index| template.replace('#', &index.to_string()))
        .collect();
    items.join(",")
}

fn accepts(bound: Bound, count: usize) -> bool {
    match bound {
        Bound::Questions => serde_json::from_str::<DecisionQuestionMap>(&format!(
            "{{{}}}",
            joined(count, r#""q#":{"type":"noul","instructions":"Urgent?"}"#)
        ))
        .is_ok(),
        Bound::ChoiceCriteria => serde_json::from_str::<systemone::ChoiceQuestion>(&format!(
            r#"{{"criteria":{{{}}}}}"#,
            joined(count, r#""c#":null"#)
        ))
        .is_ok(),
        Bound::ScoreCriteria => serde_json::from_str::<systemone::ScoreQuestion>(&format!(
            r#"{{"criteria":[{}]}}"#,
            joined(count, r#""level #""#)
        ))
        .is_ok(),
        Bound::Choices => serde_json::from_str::<openai::ChoiceQuestion>(&format!(
            r#"{{"instructions":"Lang?","choices":[{}]}}"#,
            joined(count, r#"{"value":"c#"}"#)
        ))
        .is_ok(),
        Bound::Levels => serde_json::from_str::<openai::ScoreQuestion>(&format!(
            r#"{{"instructions":"Rate","levels":[{}]}}"#,
            joined(count, r#"{"label":"l#"}"#)
        ))
        .is_ok(),
        Bound::OpenaiQuestions => serde_json::from_str::<openai::DecisionRequestBody>(&format!(
            r#"{{"input":"hi","questions":[{}]}}"#,
            joined(count, r#"{"type":"predicate","instructions":"Spam?"}"#)
        ))
        .is_ok(),
    }
}

#[rstest]
#[case::question_map(Bound::Questions, [0, 1, 128, 129])]
#[case::choice_criteria(Bound::ChoiceCriteria, [0, 1, 255, 256])]
#[case::score_criteria(Bound::ScoreCriteria, [0, 1, 10, 11])]
#[case::openai_choices(Bound::Choices, [1, 2, 255, 256])]
#[case::openai_levels(Bound::Levels, [1, 2, 10, 11])]
#[case::openai_questions(Bound::OpenaiQuestions, [0, 1, 128, 129])]
fn rejects_question_and_choice_bounds(#[case] bound: Bound, #[case] counts: [usize; 4]) {
    assert_eq!(
        counts.map(|count| accepts(bound, count)),
        [false, true, true, false]
    );
}

#[rstest]
#[case::empty_question_name(
    r#"{"state":"s","questions":{"":{"type":"noul","instructions":"Urgent?"}}}"#
)]
#[case::number_state(r#"{"state":1,"questions":{"q":{"type":"noul","instructions":"Urgent?"}}}"#)]
fn rejects_malformed_systemone_requests(#[case] json: &str) {
    serde_json::from_str::<systemone::DecisionsRequestBody>(json).unwrap_err();
}

#[rstest]
#[case::question_field(
    r#"{"input":"hi","questions":[{"type":"predicate","instructions":"Spam?","x":1}]}"#
)]
#[case::message_field(r#"{"input":[{"content":"hi","x":1}],"questions":[{"type":"predicate","instructions":"Spam?"}]}"#)]
#[case::part_field(r#"{"input":[{"content":[{"type":"input_text","text":"hi","x":1}]}],"questions":[{"type":"predicate","instructions":"Spam?"}]}"#)]
#[case::assistant_role(r#"{"input":[{"role":"assistant","content":"hi"}],"questions":[{"type":"predicate","instructions":"Spam?"}]}"#)]
fn rejects_unknown_openai_fields(#[case] json: &str) {
    serde_json::from_str::<openai::DecisionRequestBody>(json).unwrap_err();
}
