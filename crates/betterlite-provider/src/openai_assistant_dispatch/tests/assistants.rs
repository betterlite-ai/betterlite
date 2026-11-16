#![cfg(test)]

use std::sync::Arc;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::{AssistantCreate, ListParams};

use crate::openai_assistant_dispatch::OpenAiAssistantResult;
use crate::openai_assistant_dispatch::call::{
    AssistantsCall, AssistantsFacade, AssistantsOperation, OpenAiAssistantOperation,
};
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;
use crate::openai_assistant_dispatch::reply::AssistantsReply;

use super::support::{BASE, ERROR_BODY, Recorder, RecordingLogger, options, run_operation};

const ASSISTANT: &str =
    r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}"#;
const PAGE: &str = r#"{"object":"list","data":[{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[]}],"first_id":"asst_1","last_id":"asst_1","has_more":true}"#;
const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;

fn assistants_operation(operation: AssistantsOperation) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Assistants(AssistantsCall {
        options: options(),
        operation,
    })
}

fn caller_params() -> ListParams {
    ListParams {
        order: Some(String::from("asc")),
        limit: Some(5),
        before: Some(String::from("b")),
        after: Some(String::from("a")),
    }
}

async fn assistants_create() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, ASSISTANT);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Create {
        data: Box::new(
            serde_json::from_str::<AssistantCreate>(r#"{"model":"gpt-4o","name":"helper"}"#)
                .unwrap(),
        ),
        async_requested: true,
    });
    let created = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Assistants(reply) = created else {
        panic!("create did not return an assistant")
    };
    let AssistantsReply::Assistant(assistant) = *reply else {
        panic!()
    };
    assert_eq!(assistant.id, "asst_1");
    assert_eq!(assistant.model, "gpt-4o");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, format!("{BASE}/assistants"));
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-fixture"));
    assert_eq!(seen[0].organization.as_deref(), Some("org-fixture"));
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    let expected: JsonValue =
        serde_json::from_str(r#"{"model":"gpt-4o","name":"helper"}"#).unwrap();
    assert_eq!(seen[0].body.as_ref(), Some(&expected));
}

async fn assistants_delete() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, DELETED);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Delete {
        assistant_id: String::from("a/b c"),
        async_requested: false,
    });
    let removed = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Assistants(reply) = removed else {
        panic!("delete did not return a deletion")
    };
    let AssistantsReply::Deleted(deleted) = *reply else {
        panic!()
    };
    assert!(deleted.deleted);
    assert_eq!(deleted.id, "asst_1");
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "DELETE");
    assert_eq!(seen[0].url, format!("{BASE}/assistants/a%2Fb%20c"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    assert_eq!(seen[0].body, None);
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn assistants_listing_async_facade() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, PAGE);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::Facade {
            aget_assistants: true,
        },
        params: caller_params(),
    });
    let listed = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Assistants(reply) = listed else {
        panic!("the async facade listing did not return a page")
    };
    let AssistantsReply::Page(page) = *reply else {
        panic!()
    };
    assert_eq!(page.data[0].id, "asst_1");
    assert_eq!(page.has_more, Some(true));
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        format!("{BASE}/assistants?limit=20&order=desc")
    );
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn assistants_listing_sync_facade() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, PAGE);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::Facade {
            aget_assistants: false,
        },
        params: caller_params(),
    });
    run_operation(&transport, &mut logger, call).await.unwrap();
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        format!("{BASE}/assistants?after=a&before=b&limit=5&order=asc")
    );
    assert_eq!(seen[0].beta.as_deref(), Some("assistants=v2"));
}

async fn assistants_listing_helper() {
    let transport = Arc::new(Recorder::default());
    transport.queue(200, PAGE);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::List {
        facade: AssistantsFacade::AsyncHelper,
        params: caller_params(),
    });
    run_operation(&transport, &mut logger, call).await.unwrap();
    let seen = transport.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].url,
        format!("{BASE}/assistants?after=a&before=b&limit=5&order=asc")
    );
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer sk-fixture"));
}

async fn assistants_leaf_error() {
    let transport = Arc::new(Recorder::default());
    transport.queue(404, ERROR_BODY);
    let mut logger = RecordingLogger::default();
    let call = assistants_operation(AssistantsOperation::Delete {
        assistant_id: String::from("asst_missing"),
        async_requested: true,
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing assistant error did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Assistants(AssistantsError::Status { status: 404, .. })
    ));
}

pub(super) async fn case() {
    assistants_create().await;
    assistants_delete().await;
    assistants_listing_async_facade().await;
    assistants_listing_sync_facade().await;
    assistants_listing_helper().await;
    assistants_leaf_error().await;
}
