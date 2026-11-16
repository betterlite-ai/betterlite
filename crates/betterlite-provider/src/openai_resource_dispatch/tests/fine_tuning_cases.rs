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
    serde_json::from_str(r#"{"model":"gpt-test","training_file":"file-1","suffix":"v2"}"#)
        .expect("a fine-tune create request")
}

async fn run(
    call: Call<'_>,
    answer: Result<Vec<u8>, String>,
) -> (
    Result<OpenAiResourceResult, OpenAiResourceError>,
    Vec<JobCall>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let builder = NoBuilder;
    let connection = Connection {
        settings: ClientSettings::default(),
        litellm_params: None,
        client: Some(Box::new(JobSdk {
            calls: Arc::clone(&calls),
            answer,
        })),
        is_async: false,
        builder: &builder,
    };
    let operation = OpenAiResourceOperation::FineTuning(Box::new(Dispatch { connection, call }));
    let result = dispatch(operation).await;
    let recorded = calls.lock().unwrap().clone();
    (result, recorded)
}

async fn job_row(call: Call<'_>, id: &str, sent: JobCall) {
    let (result, recorded) = run(call, Ok(job_body(id))).await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Job(job))) = result else {
        panic!("the reply decodes to a job");
    };
    assert_eq!(job.id, id);
    assert_eq!(recorded, [sent]);
}

async fn list_row() {
    let page = br#"{"object":"list","data":[],"has_more":false}"#.to_vec();
    let call = Call::List {
        after: Some("ftjob-0"),
        limit: Some(4),
    };
    let (result, recorded) = run(call, Ok(page.clone())).await;
    let Ok(OpenAiResourceResult::FineTuning(Outcome::Page(bytes))) = result else {
        panic!("the list reply stays the SDK page bytes");
    };
    assert_eq!(bytes, page);
    let listed = JobCall::List {
        after: Some(String::from("ftjob-0")),
        limit: Some(4),
    };
    assert_eq!(recorded, [listed]);
}

async fn error_row() {
    let answer = Err(String::from("the training file is missing"));
    let (result, recorded) = run(Call::Retrieve("ftjob-1"), answer).await;
    let Err(error) = result else {
        panic!("the SDK status error is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::FineTuning(FineTuneError::Sdk { .. })
    ));
    assert_eq!(recorded, [JobCall::Retrieve(String::from("ftjob-1"))]);
}

pub(crate) async fn check() {
    let data = create_data();
    let created = JobCall::Create {
        model: String::from("gpt-test"),
        training_file: String::from("file-1"),
        suffix: Some(String::from("v2")),
    };
    job_row(Call::Create(&data), "ftjob-created", created).await;
    let retrieved = JobCall::Retrieve(String::from("ftjob-1"));
    job_row(Call::Retrieve("ftjob-1"), "ftjob-retrieved", retrieved).await;
    let cancelled = JobCall::Cancel(String::from("ftjob-1"));
    job_row(Call::Cancel("ftjob-1"), "ftjob-cancelled", cancelled).await;
    list_row().await;
    error_row().await;
}
