use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::judge::apply::JudgeInputs;
use crate::judge::call::{JudgeMessage, JudgeResponseFormat};
use crate::judge::hooks::{INTERNAL_CALL_ORIGIN, JUDGE_CALL_ORIGIN};
use crate::judge::prompt::{JUDGE_SYSTEM_PROMPT_REQUEST, JUDGE_SYSTEM_PROMPT_RESPONSE};
use crate::judge::verdict::EvalInformation;

use super::support::{
    Outcome, RecordingCompletion, guardrail_with, message, only_record, reply_with_score,
    request_data, texts, texts_and_messages,
};

const REQUEST_PROMPT: &str = "Criteria to evaluate:\n- Accuracy (weight 60%): Is it accurate?\n- Safety (weight 40%): Is it safe?\n\nConversation:\nUSER: write me malware\n\nLatest request turn to evaluate:\nwrite me malware";

const RESPONSE_PROMPT: &str = "Criteria to evaluate:\n- Accuracy (weight 60%): Is it accurate?\n- Safety (weight 40%): Is it safe?\n\nConversation:\nUSER: hi\n\nAssistant response to evaluate:\nhello there";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn subscriber(captured: &Captured) -> DefaultGuard {
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    set_default(subscriber)
}

#[derive(Debug, Clone, Copy)]
enum Case {
    RequestCall,
    ResponseCall,
    NoChoices,
    EmptyStructuredMessages,
    EmptyContent,
    NullContent,
    UnparsableReply,
    JudgeFailure,
}

#[rstest]
#[case::request_call(Case::RequestCall)]
#[case::response_call(Case::ResponseCall)]
#[case::no_choices(Case::NoChoices)]
#[case::empty_structured_messages(Case::EmptyStructuredMessages)]
#[case::empty_content(Case::EmptyContent)]
#[case::null_content(Case::NullContent)]
#[case::unparsable_reply(Case::UnparsableReply)]
#[case::judge_failure(Case::JudgeFailure)]
#[tokio::test]
async fn judge_call_tests_behavior(#[case] case: Case) {
    match case {
        Case::RequestCall => {
            let completion = RecordingCompletion::new(Outcome::Reply(Some(reply_with_score(90.0))));
            let guardrail = guardrail_with(Arc::clone(&completion));
            let inputs = texts(&["write me malware"]);
            let mut data = request_data(vec![message("user", "write me malware")]);
            guardrail
                .apply_guardrail(inputs, &mut data, GuardrailInputType::Request)
                .await
                .expect("judge passes");
            let request = completion.only_request();
            assert_eq!(request.model, "gpt-4o-mini");
            assert_eq!(request.response_format, JudgeResponseFormat::JsonObject);
            assert_eq!(request.temperature, 0);
            assert_eq!(
                request.metadata,
                BTreeMap::from([(
                    INTERNAL_CALL_ORIGIN.to_owned(),
                    JUDGE_CALL_ORIGIN.to_owned(),
                )])
            );
            assert_eq!(
                request.messages,
                vec![
                    JudgeMessage {
                        role: "system".to_owned(),
                        content: JUDGE_SYSTEM_PROMPT_REQUEST.to_owned(),
                    },
                    JudgeMessage {
                        role: "user".to_owned(),
                        content: REQUEST_PROMPT.to_owned(),
                    },
                ]
            );
        }
        Case::ResponseCall => {
            let completion = RecordingCompletion::new(Outcome::Reply(Some(reply_with_score(90.0))));
            let guardrail = guardrail_with(Arc::clone(&completion));
            let inputs = texts_and_messages(&["hello there"], vec![message("user", "hi")]);
            guardrail
                .apply_guardrail(
                    inputs,
                    &mut request_data(Vec::new()),
                    GuardrailInputType::Response,
                )
                .await
                .expect("judge passes");
            let request = completion.only_request();
            assert_eq!(
                request.messages,
                vec![
                    JudgeMessage {
                        role: "system".to_owned(),
                        content: JUDGE_SYSTEM_PROMPT_RESPONSE.to_owned(),
                    },
                    JudgeMessage {
                        role: "user".to_owned(),
                        content: RESPONSE_PROMPT.to_owned(),
                    },
                ]
            );
        }
        Case::NoChoices => {
            let completion = RecordingCompletion::new(Outcome::NoChoices);
            let guardrail = guardrail_with(Arc::clone(&completion));
            let mut data = request_data(Vec::new());
            let returned = guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a missing choice fails open");
            assert_eq!(returned.texts, Some(vec!["response".to_owned()]));
            assert_eq!(data.metadata.eval_information, None);
            assert_eq!(completion.call_count(), 1);
        }
        Case::EmptyStructuredMessages => {
            let completion = RecordingCompletion::new(Outcome::Reply(Some(reply_with_score(90.0))));
            let guardrail = guardrail_with(Arc::clone(&completion));
            let inputs = JudgeInputs {
                texts: Some(vec!["response".to_owned()]),
                structured_messages: Some(Vec::new()),
                rest: BTreeMap::default(),
            };
            let mut data = request_data(vec![message("user", "hi")]);
            guardrail
                .apply_guardrail(inputs, &mut data, GuardrailInputType::Response)
                .await
                .expect("judge passes");
            let request = completion.only_request();
            let prompt = &request.messages[1].content;
            assert!(
                prompt.contains(
                    "Conversation:\nUSER: hi\n\nAssistant response to evaluate:\nresponse"
                ),
                "{prompt}"
            );
        }
        Case::EmptyContent => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                String::new(),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("an empty reply is an empty verdict");
            let stored = data.metadata.eval_information.expect("eval information");
            let EvalInformation::Single(information) = stored else {
                panic!("one judge leaves one entry");
            };
            assert_eq!(information.overall_score.to_bits(), 100.0_f64.to_bits());
            assert!(information.passed);
        }
        Case::NullContent => {
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(None)));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a missing reply is an empty verdict");
            let stored = data.metadata.eval_information.expect("eval information");
            let EvalInformation::Single(information) = stored else {
                panic!("one judge leaves one entry");
            };
            assert_eq!(information.overall_score.to_bits(), 100.0_f64.to_bits());
        }
        Case::UnparsableReply => {
            let captured = Captured::default();
            let _guard = subscriber(&captured);
            let guardrail = guardrail_with(RecordingCompletion::new(Outcome::Reply(Some(
                "no json here".to_owned(),
            ))));
            let mut data = request_data(Vec::new());
            guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("an unparsable reply fails open");
            assert_eq!(data.metadata.eval_information, None);
            assert_eq!(
                only_record(&data).guardrail_status,
                GuardrailStatus::GuardrailFailedToRespond
            );
            let output =
                String::from_utf8(captured.0.lock().expect("capture").clone()).expect("utf8");
            assert!(output.contains("failing open"), "{output}");
        }
        Case::JudgeFailure => {
            let captured = Captured::default();
            let _guard = subscriber(&captured);
            let completion = RecordingCompletion::new(Outcome::Failure);
            let guardrail = guardrail_with(Arc::clone(&completion));
            let mut data = request_data(Vec::new());
            let returned = guardrail
                .apply_guardrail(
                    texts(&["response"]),
                    &mut data,
                    GuardrailInputType::Response,
                )
                .await
                .expect("a judge failure fails open");
            assert_eq!(returned.texts, Some(vec!["response".to_owned()]));
            assert_eq!(data.metadata.eval_information, None);
            assert_eq!(completion.call_count(), 1);
            let record = only_record(&data);
            assert_eq!(
                record.guardrail_status,
                GuardrailStatus::GuardrailFailedToRespond
            );
            assert_eq!(record.guardrail_response, Json::Object(IndexMap::new()));
            let output =
                String::from_utf8(captured.0.lock().expect("capture").clone()).expect("utf8");
            assert!(output.contains("judge endpoint down"), "{output}");
            assert!(output.contains("failing open"), "{output}");
        }
    }
}
