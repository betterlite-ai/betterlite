#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::records::{AssistantCreate, AssistantObject};

use crate::azure_assistant_definitions::operations::{DefinitionCall, DefinitionReply};

use super::support::{ASSISTANT, ASSISTANTS_URL, Auth, CREATE_BODY, Fixture, KEY, harness, wire};

// A key the caller sent as null stays null in the request; one the caller left out stays out.
const NULL_BODY: &str = r#"{"model":"gpt-4o","name":null,"description":null,"tool_resources":null,"tools":[{"type":"file_search","file_search":null}],"metadata":null}"#;

pub(super) async fn check() {
    sync_create().await;
    async_create().await;
    null_fields().await;
}

async fn sync_create() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (api, transport) = harness(200, ASSISTANT);
    let created = api
        .call(&request, None, Some(false), DefinitionCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(created.kind, ClientKind::Sync);
    let DefinitionReply::Assistant(assistant) = created.reply else {
        panic!("a create call returns an assistant");
    };
    let expected: AssistantObject = serde_json::from_str(ASSISTANT).unwrap();
    assert_eq!(*assistant, expected);
    assert_eq!(
        transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(CREATE_BODY),
            Auth::Key(KEY)
        )]
    );
}

async fn async_create() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (api, transport) = harness(200, ASSISTANT);
    let awaited = api
        .call(&request, None, Some(true), DefinitionCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(awaited.kind, ClientKind::Asynchronous);
    assert_eq!(
        transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(CREATE_BODY),
            Auth::Key(KEY)
        )]
    );
}

async fn null_fields() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let nulls: AssistantCreate = serde_json::from_str(NULL_BODY).unwrap();
    let (api, transport) = harness(200, ASSISTANT);
    api.call(&request, None, None, DefinitionCall::Create(&nulls))
        .await
        .unwrap();
    assert_eq!(
        transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(NULL_BODY),
            Auth::Key(KEY)
        )]
    );
}
