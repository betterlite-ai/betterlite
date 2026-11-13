#![cfg(test)]

pub(crate) mod support;

mod batch;

use std::fs::read_to_string;
use std::sync::Arc;
use std::time::SystemTime;

use betterlite_context::call_context::BillingPolicy;
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::{CallLog as _, CallOutcome, CallRecord, TracingCallLog};
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

#[rstest]
#[case::success(CallOutcome::Succeeded, None, "call finished")]
#[case::cancelled(CallOutcome::Cancelled, None, "client disconnected, call cancelled")]
#[case::failed_with_message(CallOutcome::Failed, Some("stream broke"), "stream broke")]
#[case::failed_with_error(CallOutcome::Failed, None, "does not dispatch the chat call yet")]
fn the_tracing_log_writes_each_outcome(
    #[case] outcome: CallOutcome,
    #[case] message: Option<&str>,
    #[case] expected: &str,
) {
    let file = NamedTempFile::new().expect("log file");
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(file.reopen().expect("log writer")))
        .finish();
    let error = DispatchError::NotBuilt { entry: "chat" };
    with_default(subscriber, || {
        TracingCallLog.record(CallRecord {
            entry: Entry::Chat,
            call_id: "call-9",
            billing: BillingPolicy::Include,
            privacy: false,
            outcome,
            error: Some(&error),
            message,
        });
    });
    let output = read_to_string(file.path()).expect("log text");
    assert!(output.contains(expected), "{output}");
    assert!(output.contains("entry=\"chat\""), "{output}");
    assert!(output.contains("call_id=\"call-9\""), "{output}");
}

#[test]
fn every_entry_has_a_distinct_name() {
    let mut names: Vec<&str> = Entry::ALL
        .iter()
        .map(|entry| <&str>::from(*entry))
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Entry::ALL.len());
    assert_eq!(Entry::ALL.len(), 19);
}

#[rstest]
#[case::chat(&[CallType::Completion, CallType::Acompletion], Some(Entry::Chat))]
#[case::embedding(&[CallType::Embedding, CallType::Aembedding], Some(Entry::Embedding))]
#[case::text_completion(
    &[CallType::TextCompletion, CallType::AtextCompletion],
    Some(Entry::TextCompletion)
)]
#[case::moderation(&[CallType::Moderation, CallType::Amoderation], Some(Entry::Moderation))]
#[case::responses(&[CallType::Responses, CallType::Aresponses], Some(Entry::Responses))]
#[case::image_generation(
    &[CallType::ImageGeneration, CallType::AimageGeneration],
    Some(Entry::ImageGeneration)
)]
#[case::image_edit(&[CallType::ImageEdit, CallType::AimageEdit], Some(Entry::ImageEdit))]
#[case::video_generation(
    &[CallType::VideoGeneration, CallType::AvideoGeneration],
    Some(Entry::VideoGeneration)
)]
#[case::video_list(&[CallType::VideoList, CallType::AvideoList], Some(Entry::VideoList))]
#[case::video_status(
    &[CallType::VideoRetrieve, CallType::AvideoRetrieve],
    Some(Entry::VideoStatus)
)]
#[case::video_content(
    &[CallType::VideoContent, CallType::AvideoContent],
    Some(Entry::VideoContent)
)]
#[case::video_remix(&[CallType::VideoRemix, CallType::AvideoRemix], Some(Entry::VideoRemix))]
#[case::video_create_character(
    &[CallType::VideoCreateCharacter, CallType::AvideoCreateCharacter],
    Some(Entry::VideoCreateCharacter)
)]
#[case::video_get_character(
    &[CallType::VideoGetCharacter, CallType::AvideoGetCharacter],
    Some(Entry::VideoGetCharacter)
)]
#[case::video_edit(&[CallType::VideoEdit, CallType::AvideoEdit], Some(Entry::VideoEdit))]
#[case::video_extension(
    &[CallType::VideoExtension, CallType::AvideoExtension],
    Some(Entry::VideoExtension)
)]
#[case::passthrough(
    &[CallType::LlmPassthroughRoute, CallType::AllmPassthroughRoute],
    Some(Entry::LlmPassthrough)
)]
#[case::container(
    &[
        CallType::CreateContainer,
        CallType::AcreateContainer,
        CallType::ListContainers,
        CallType::AlistContainers,
        CallType::RetrieveContainer,
        CallType::AretrieveContainer,
        CallType::DeleteContainer,
        CallType::AdeleteContainer,
        CallType::ListContainerFiles,
        CallType::AlistContainerFiles,
        CallType::UploadContainerFile,
        CallType::AuploadContainerFile,
    ],
    Some(Entry::ContainerCall)
)]
#[case::no_entry(&[CallType::Asearch], None)]
fn call_type_recognition_tests_behavior(
    #[case] types: &[CallType],
    #[case] expected: Option<Entry>,
) {
    for call_type in types {
        assert_eq!(call_types::entry_for(*call_type), expected, "{call_type:?}");
    }
}
