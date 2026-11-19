use std::time::Duration;

use betterlite_openai::files::records::{
    CreatePurpose, ExpiresAfter, FileContentRequest, FileCreate, FileDeleted, FileKind, FileObject,
    FilePage, FileUpload, FilesPurpose,
};
use http::Method;
use indexmap::IndexMap;
use rstest::rstest;

use super::inputs::{azure_file_client_errors, azure_file_client_inputs, azure_file_routes};
use super::*;

#[rstest]
#[case::file_upload(file_upload)]
#[case::file_content(file_content)]
#[case::file_retrieve(file_retrieve)]
#[case::file_list(file_list)]
#[case::file_delete(file_delete)]
#[case::azure_file_client_inputs(azure_file_client_inputs)]
#[case::azure_file_client_errors(azure_file_client_errors)]
#[case::azure_file_routes(azure_file_routes)]
fn azure_files_cases(#[case] case: fn()) {
    case();
}

fn field(name: &str, text: &str) -> (String, Option<String>, Option<String>, String) {
    (name.to_owned(), None, None, text.to_owned())
}

fn file_part(
    filename: &str,
    content_type: &str,
    text: &str,
) -> (String, Option<String>, Option<String>, String) {
    (
        "file".to_owned(),
        Some(filename.to_owned()),
        Some(content_type.to_owned()),
        text.to_owned(),
    )
}

fn full_upload() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: Some("notes.txt".to_owned()),
            content_type: None,
            data: b"secret-bytes".to_vec(),
        },
        purpose: CreatePurpose::Batch,
        expires_after: Some(ExpiresAfter { seconds: 3600 }),
        extra_headers: IndexMap::from([("x-trace".to_owned(), "t1".to_owned())]),
        extra_body: IndexMap::from([("note".to_owned(), "n1".to_owned())]),
        timeout: Some(Duration::from_millis(7500)),
    }
}

fn plain_upload() -> FileCreate {
    FileCreate {
        file: FileUpload {
            filename: None,
            content_type: None,
            data: b"plain".to_vec(),
        },
        purpose: CreatePurpose::Assistants,
        expires_after: None,
        extra_headers: IndexMap::new(),
        extra_body: IndexMap::new(),
        timeout: None,
    }
}

fn object_of(text: &str) -> FileObject {
    serde_json::from_str(text).expect("the expected file object parses")
}

fn created(step: Step, data: FileCreate) -> Observation {
    run(Step {
        data: Some(data),
        reply: Some(json_reply(FILE_JSON)),
        ..step
    })
}

fn file_upload() {
    let data = full_upload();
    let original = data.clone();
    let observation = run(Step {
        data: Some(data.clone()),
        reply: Some(json_reply(FILE_JSON)),
        ..step(Action::Create)
    });
    assert_eq!(observation.outcome, Outcome::Object(object_of(FILE_JSON)));
    assert_eq!(data, original, "the request data is left as it was");
    upload_request(only_call(&observation));
    upload_logs();
    file_upload_caller_headers_win();
    file_upload_supplied_client_keeps_caller_authorization();
}

fn file_upload_caller_headers_win() {
    let mut data = plain_upload();
    data.extra_headers = IndexMap::from([
        ("api-key".to_owned(), "caller-key".to_owned()),
        ("authorization".to_owned(), "Bearer caller".to_owned()),
    ]);
    let request = only_call(&created(step(Action::Create), data));
    assert_eq!(header(request, "api-key"), Some("caller-key"));
    assert_eq!(header(request, "authorization"), Some("Bearer caller"));
}

fn file_upload_supplied_client_keeps_caller_authorization() {
    let mut data = plain_upload();
    data.extra_headers = IndexMap::from([("authorization".to_owned(), "Bearer caller".to_owned())]);
    let observation = run(Step {
        client: Some(Supplied::OpenAi),
        data: Some(data),
        reply: Some(json_reply(FILE_JSON)),
        ..step(Action::Create)
    });
    assert_eq!(
        header(only_call(&observation), "authorization"),
        Some("Bearer caller")
    );
}

fn upload_request(request: &OutboundRequest) {
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.path(), "/openai/files");
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );
    assert_eq!(header(request, "api-key"), Some(KEY));
    assert_eq!(header(request, "authorization"), None);
    assert_eq!(header(request, "accept"), Some("application/json"));
    assert_eq!(header(request, "x-trace"), Some("t1"));
    assert_eq!(request.deadline, Duration::from_millis(7500));
    assert_eq!(
        part_view(request),
        vec![
            field("purpose", "batch"),
            field("expires_after[anchor]", "created_at"),
            field("expires_after[seconds]", "3600"),
            field("note", "n1"),
            file_part("notes.txt", "text/plain", "secret-bytes"),
        ]
    );
}

fn upload_logs() {
    let data = full_upload();
    let async_run = run(Step {
        is_async: true,
        data: Some(data.clone()),
        reply: Some(json_reply(FILE_JSON)),
        ..step(Action::Create)
    });
    let lines: Vec<&String> = async_run
        .messages
        .iter()
        .filter(|line| line.contains("create_file_data") || line.contains("create_file_response"))
        .collect();
    assert_eq!(lines.len(), 2, "one request record and one response record");
    let request_line = lines
        .iter()
        .find(|line| line.contains(DEBUG_CREATE))
        .expect("the request record is logged");
    assert!(request_line.contains("filename=notes.txt"));
    assert!(request_line.contains("purpose=batch"));
    assert!(request_line.contains("bytes=12"));
    assert!(
        !request_line.contains("secret-bytes"),
        "the body is redacted"
    );
    assert!(!request_line.contains(KEY), "the credential is redacted");
    let response_line = lines
        .iter()
        .find(|line| line.contains(DEBUG_RESPONSE))
        .expect("the response record is logged");
    assert!(response_line.contains("id=file-1"));
    let sync_run = run(Step {
        data: Some(data),
        reply: Some(json_reply(FILE_JSON)),
        ..step(Action::Create)
    });
    assert!(
        !sync_run
            .messages
            .iter()
            .any(|line| line.contains(DEBUG_CREATE) || line.contains(DEBUG_RESPONSE)),
        "the sync path logs neither record"
    );
}

fn file_upload_defaults() {
    let observation = created(step(Action::Create), plain_upload());
    assert_eq!(observation.outcome, Outcome::Object(object_of(FILE_JSON)));
    let request = only_call(&observation);
    assert_eq!(request.deadline, Duration::from_secs(600));
    assert_eq!(
        part_view(request),
        vec![
            field("purpose", "assistants"),
            file_part("upload", "application/octet-stream", "plain"),
        ]
    );
}

fn file_upload_keeps_expiry_only_when_given() {
    let mut data = plain_upload();
    data.expires_after = Some(ExpiresAfter { seconds: 1 });
    let observation = created(step(Action::Create), data);
    let parts = part_view(only_call(&observation));
    assert!(parts.contains(&field("expires_after[anchor]", "created_at")));
    assert!(parts.contains(&field("expires_after[seconds]", "1")));
    let mut absent = plain_upload();
    absent.extra_body = IndexMap::from([("note".to_owned(), "n".to_owned())]);
    let observation = created(step(Action::Create), absent);
    let parts = part_view(only_call(&observation));
    assert!(!parts.iter().any(|part| part.0.starts_with("expires_after")));
}

fn file_upload_validates_the_reply() {
    let incomplete = r#"{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","status":"processed"}"#;
    let observation = run(Step {
        data: Some(plain_upload()),
        reply: Some(json_reply(incomplete)),
        ..step(Action::Create)
    });
    assert_eq!(
        observation.outcome,
        Outcome::Raised(Raised {
            class: "ValidationError",
            message: None,
            status: None,
            error_type: None,
            param: None,
            code: None,
        })
    );
}

fn content_step(reply: Planned) -> Step {
    Step {
        content: Some(FileContentRequest {
            file_id: "file two".to_owned(),
            extra_headers: IndexMap::from([("x-trace".to_owned(), "t1".to_owned())]),
            extra_body: IndexMap::from([("note".to_owned(), "n1".to_owned())]),
            timeout: Some(Duration::from_millis(3500)),
        }),
        reply: Some(reply),
        ..step(Action::Content)
    }
}

fn file_content() {
    let mut reply = reply(200, "file-bytes");
    reply.headers.push(("content-type", "application/binary"));
    reply.headers.push(("x-request-id", "r1"));
    let observation = run(content_step(reply));
    let Outcome::Content(content) = observation.outcome else {
        panic!("the content call returns the body");
    };
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"file-bytes");
    assert_eq!(content.headers.get("x-request-id"), Some(&"r1".to_owned()));
    let request = only_call(&observation);
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), "/openai/files/file%20two/content");
    assert_eq!(header(request, "accept"), Some("application/binary"));
    assert_eq!(header(request, "x-trace"), Some("t1"));
    assert_eq!(request.deadline, Duration::from_millis(3500));
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), VERSION.to_owned())],
        "extra_body never reaches a GET"
    );
}

fn file_content_replies() {
    let empty = run(content_step(reply(200, "")));
    assert_eq!(
        empty.outcome,
        Outcome::Content(BinaryContent {
            status: 200,
            headers: IndexMap::new(),
            body: Vec::new(),
        })
    );
    let missing = run(content_step(reply(404, STATUS_ERROR)));
    assert_eq!(
        missing.outcome,
        Outcome::Raised(Raised {
            class: "APIStatusError",
            message: Some(format!("Error code: 404 - {STATUS_ERROR}")),
            status: Some(404),
            error_type: Some("invalid_request_error".to_owned()),
            param: Some("file_id".to_owned()),
            code: Some("not_found".to_owned()),
        })
    );
}

fn file_retrieve() {
    let with_extra = r#"{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","purpose":"batch","status":"processed","extra":1}"#;
    let observation = run(Step {
        file_id: Some("file two".to_owned()),
        reply: Some(json_reply(with_extra)),
        ..step(Action::Retrieve)
    });
    assert_eq!(observation.outcome, Outcome::Object(object_of(FILE_JSON)));
    let request = only_call(&observation);
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), "/openai/files/file%20two");
    assert_eq!(header(request, "accept"), Some("application/json"));
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );
}

fn file_retrieve_keeps_optional_fields() {
    let minimal = r#"{"id":"file-1","object":"file","bytes":5,"created_at":1700000000,"filename":"notes.txt","purpose":"assistants_output"}"#;
    let observation = run(Step {
        file_id: Some("file-1".to_owned()),
        reply: Some(json_reply(minimal)),
        ..step(Action::Retrieve)
    });
    let Outcome::Object(object) = observation.outcome else {
        panic!("the retrieve call returns the SDK object");
    };
    assert_eq!(object.id, "file-1");
    assert_eq!(object.purpose, FilesPurpose::AssistantsOutput);
    assert_eq!(object.status, None);
    assert_eq!(object.expires_at, None);
    assert_eq!(object.status_details, None);
}

fn list_step(purpose: Option<&str>, reply: Planned) -> Step {
    Step {
        purpose: purpose.map(str::to_owned),
        reply: Some(reply),
        ..step(Action::List)
    }
}

fn file_list() {
    let observation = run(list_step(Some("batch"), json_reply(PAGE_JSON)));
    assert_eq!(
        observation.outcome,
        Outcome::Page(serde_json::from_str(PAGE_JSON).expect("the expected page parses"))
    );
    let request = only_call(&observation);
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url.path(), "/openai/files");
    assert_eq!(
        request.query,
        vec![
            ("api-version".to_owned(), VERSION.to_owned()),
            ("purpose".to_owned(), "batch".to_owned()),
        ]
    );
}

fn file_list_purpose_forms() {
    let empty = run(list_step(Some(""), json_reply(PAGE_JSON)));
    assert_eq!(
        only_call(&empty).query,
        vec![("api-version".to_owned(), VERSION.to_owned())],
        "an empty purpose is not sent"
    );
    let absent = run(list_step(None, json_reply(PAGE_JSON)));
    assert_eq!(
        only_call(&absent).query,
        vec![("api-version".to_owned(), VERSION.to_owned())]
    );
}

fn file_list_keeps_the_page() {
    let page =
        r#"{"object":"list","data":[],"has_more":true,"first_id":"file-1","last_id":"file-2"}"#;
    let observation = run(list_step(Some("batch"), json_reply(page)));
    let Outcome::Page(page) = observation.outcome else {
        panic!("the list call returns the page");
    };
    assert_eq!(page.has_more, Some(true));
    assert_eq!(page.first_id.as_deref(), Some("file-1"));
    assert_eq!(page.last_id.as_deref(), Some("file-2"));
    assert!(page.data.is_empty());
}

fn delete_step(body: &str) -> Step {
    Step {
        file_id: Some("file-1".to_owned()),
        reply: Some(json_reply(body)),
        ..step(Action::Delete)
    }
}

fn synthesized() -> FileDeleted {
    FileDeleted {
        id: "file-1".to_owned(),
        object: FileKind::File,
        deleted: true,
    }
}

fn file_delete() {
    let empty_string = run(delete_step(r#""""#));
    assert_eq!(empty_string.outcome, Outcome::Deleted(synthesized()));
    let request = only_call(&empty_string);
    assert_eq!(request.method, Method::DELETE);
    assert_eq!(request.url.path(), "/openai/files/file-1");
    assert_eq!(header(request, "accept"), Some("application/json"));
    assert_eq!(header(request, "openai-organization"), None);
    let null = run(delete_step("null"));
    assert_eq!(null.outcome, Outcome::Deleted(synthesized()));
}

fn file_delete_keeps_a_real_record() {
    let body = r#"{"id":"server-file","object":"file","deleted":false,"extra":1}"#;
    let observation = run(delete_step(body));
    assert_eq!(
        observation.outcome,
        Outcome::Deleted(FileDeleted {
            id: "server-file".to_owned(),
            object: FileKind::File,
            deleted: false,
        })
    );
}

fn file_delete_replaces_everything_else() {
    for body in ["{}", "", "not json"] {
        let observation = run(delete_step(body));
        assert_eq!(
            observation.outcome,
            Outcome::Deleted(synthesized()),
            "the body {body:?} is not a FileDeleted"
        );
    }
}

fn file_delete_status_error() {
    let observation = run(delete_step(STATUS_ERROR));
    let Outcome::Raised(raised) = observation.outcome else {
        panic!("a 404 reply raises");
    };
    assert_eq!(raised.class, "APIStatusError");
    assert_eq!(raised.status, Some(404));
    assert_eq!(raised.code.as_deref(), Some("not_found"));
}

fn file_delete_organization_is_unused() {
    let mut step = delete_step(r#""""#);
    step.settings.api_base = Some("https://fixture.test".to_owned());
    let observation = run(step);
    let request = only_call(&observation);
    assert_eq!(header(request, "openai-organization"), None);
}

#[test]
fn file_upload_empty_and_optional_forms() {
    file_upload_defaults();
    file_upload_keeps_expiry_only_when_given();
    file_upload_validates_the_reply();
}

#[test]
fn file_content_forms() {
    file_content_replies();
}

#[test]
fn file_retrieve_forms() {
    file_retrieve_keeps_optional_fields();
}

#[test]
fn file_list_forms() {
    file_list_purpose_forms();
    file_list_keeps_the_page();
}

#[test]
fn file_delete_forms() {
    file_delete_keeps_a_real_record();
    file_delete_replaces_everything_else();
    file_delete_status_error();
    file_delete_organization_is_unused();
}
