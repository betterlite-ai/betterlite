use std::sync::{Arc, Mutex};

use betterlite_openai::fine_tuning::client::{ClientSettings, Connection};
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneCreate;

use super::fixture::{JobCall, JobSdk, NoBuilder, job_body};
use crate::openai_resource_dispatch::fine_tuning::{Call, Dispatch, Outcome};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

fn create_data() -> FineTuneCreate {
    FineTuneCreate {
        model: String::from("gpt-test"),
        training_file: String::from("file-1"),
        suffix: Some(String::from("v2")),
        ..FineTuneCreate::default()
    }
}

fn connection<'a>(
    calls: &Arc<Mutex<Vec<JobCall>>>,
    builder: &'a NoBuilder,
    answer: &Result<Vec<u8>, String>,
) -> Connection<'a> {
    Connection {
        settings: ClientSettings::default(),
        litellm_params: None,
        client: Some(Box::new(JobSdk {
            calls: Arc::clone(calls),
            answer: answer.clone(),
        })),
        is_async: false,
        builder,
    }
}

async fn run(
    connection: Connection<'_>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::FineTuning(Box::new(Dispatch { connection, call }));
    dispatch(operation).await
}

async fn create_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let answer = Ok(job_body("ftjob-created"));
    let connection = connection(&calls, &builder, &answer);
    let data = create_data();
    let result = run(connection, Call::Create(&data)).await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Job(job))) = result else {
        panic!("the create reply decodes to a job");
    };
    assert_eq!(job.id, "ftjob-created");
    let recorded = calls.lock().unwrap();
    let JobCall::Create(sent) = &recorded[0] else {
        panic!("the create call reaches the SDK");
    };
    assert_eq!(sent.model, "gpt-test");
    assert_eq!(sent.training_file, "file-1");
    assert_eq!(sent.suffix.as_deref(), Some("v2"));
}

async fn retrieve_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let answer = Ok(job_body("ftjob-retrieved"));
    let connection = connection(&calls, &builder, &answer);
    let result = run(connection, Call::Retrieve("ftjob-1")).await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Job(job))) = result else {
        panic!("the retrieve reply decodes to a job");
    };
    assert_eq!(job.id, "ftjob-retrieved");
    let recorded = calls.lock().unwrap();
    assert!(matches!(&recorded[0], JobCall::Retrieve(id) if id == "ftjob-1"));
}

async fn cancel_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let answer = Ok(job_body("ftjob-cancelled"));
    let connection = connection(&calls, &builder, &answer);
    let result = run(connection, Call::Cancel("ftjob-1")).await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Job(job))) = result else {
        panic!("the cancel reply decodes to a job");
    };
    assert_eq!(job.id, "ftjob-cancelled");
    let recorded = calls.lock().unwrap();
    assert!(matches!(&recorded[0], JobCall::Cancel(id) if id == "ftjob-1"));
}

async fn list_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let page = br#"{"object":"list","data":[],"has_more":false}"#.to_vec();
    let answer = Ok(page.clone());
    let connection = connection(&calls, &builder, &answer);
    let result = run(
        connection,
        Call::List {
            after: Some("ftjob-0"),
            limit: Some(4),
        },
    )
    .await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Page(bytes))) = result else {
        panic!("the list reply stays the SDK page bytes");
    };
    assert_eq!(bytes, page);
    let recorded = calls.lock().unwrap();
    let JobCall::List { after, limit } = &recorded[0] else {
        panic!("the list call reaches the SDK");
    };
    assert_eq!(after.as_deref(), Some("ftjob-0"));
    assert_eq!(*limit, Some(4));
}

async fn error_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let answer = Err(String::from("the training file is missing"));
    let connection = connection(&calls, &builder, &answer);
    let result = run(connection, Call::Retrieve("ftjob-1")).await;
    let Err(error) = result else {
        panic!("the SDK status error is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::FineTuning(FineTuneError::Sdk { .. })
    ));
}

pub(crate) async fn check() {
    create_row().await;
    retrieve_row().await;
    cancel_row().await;
    list_row().await;
    error_row().await;
}
