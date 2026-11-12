#![cfg(test)]

pub(crate) mod support;

mod batch;

use std::sync::Arc;
use std::time::SystemTime;

use rstest::rstest;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::TracingCallLog;
use crate::dispatch::{Dispatch, Entry, NoRules};
use crate::operation::call_types;
use crate::operation::{Operation, video_list::VideoListRequest};
use crate::routes::CallType;

use support::context;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Unknown,
    Mismatched,
}

#[rstest]
#[case::unknown_call_type(Scenario::Unknown)]
#[case::mismatched_call_type(Scenario::Mismatched)]
#[tokio::test]
async fn call_dispatch_tests_behavior(#[case] scenario: Scenario) {
    let dispatch = Dispatch::new(Vec::new(), Arc::new(NoRules), Arc::new(TracingCallLog));
    let context = context(None, SystemTime::UNIX_EPOCH);
    let operation = Operation::VideoList(VideoListRequest);
    let call = match scenario {
        Scenario::Unknown => {
            dispatch
                .execute_for(CallType::Atranscription, operation, &context)
                .await
        }
        Scenario::Mismatched => {
            dispatch
                .execute_for(CallType::Aembedding, operation, &context)
                .await
        }
    };
    let Err(error) = call else {
        panic!("the call must fail before any entry runs");
    };
    let matched = matches!(
        (scenario, &error),
        (
            Scenario::Unknown,
            DispatchError::UnknownOperation {
                call_type: CallType::Atranscription,
            },
        ) | (
            Scenario::Mismatched,
            DispatchError::MismatchedOperation {
                call_type: CallType::Aembedding,
                entry: "video_list",
            },
        )
    );
    assert!(matched, "unexpected dispatch error: {error}");
}

#[test]
fn every_entry_has_a_distinct_name() {
    let mut names: Vec<&str> = Entry::ALL.iter().map(|entry| entry.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Entry::ALL.len());
    assert_eq!(Entry::ALL.len(), 19);
}

#[rstest]
#[case::chat(CallType::Acompletion, Some(Entry::Chat))]
#[case::embedding(CallType::Aembedding, Some(Entry::Embedding))]
#[case::text_completion(CallType::TextCompletion, Some(Entry::TextCompletion))]
#[case::moderation(CallType::Amoderation, Some(Entry::Moderation))]
#[case::responses(CallType::Responses, Some(Entry::Responses))]
#[case::image_generation(CallType::ImageGeneration, Some(Entry::ImageGeneration))]
#[case::video_status(CallType::VideoRetrieve, Some(Entry::VideoStatus))]
#[case::passthrough(CallType::AllmPassthroughRoute, Some(Entry::LlmPassthrough))]
#[case::container(CallType::AuploadContainerFile, Some(Entry::ContainerCall))]
#[case::no_entry(CallType::Asearch, None)]
fn call_type_recognition_tests_behavior(
    #[case] call_type: CallType,
    #[case] expected: Option<Entry>,
) {
    assert_eq!(call_types::entry_for(call_type), expected);
}
