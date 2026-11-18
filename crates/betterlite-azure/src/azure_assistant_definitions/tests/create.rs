#![cfg(test)]

use betterlite_openai::assistants::client::ClientKind;
use betterlite_openai::assistants::records::AssistantCreate;

use crate::azure_assistant_definitions::operations::{DefinitionCall, DefinitionReply};

use super::support::{
    ASSISTANT, ASSISTANTS_URL, Auth, CREATE_BODY, Fixture, KEY, harness, parse, wire,
};

// A key the caller sent as null stays null in the request; one the caller left out stays out.
const NULL_BODY: &str = r#"{"model":"gpt-4o","name":null,"description":null,"tool_resources":null,"tools":[{"type":"file_search","file_search":null}],"metadata":null}"#;

pub(super) async fn check() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let data: AssistantCreate = serde_json::from_str(CREATE_BODY).unwrap();
    let (sync_api, sync_transport) = harness(200, ASSISTANT);
    let created = sync_api
        .call(&request, None, Some(false), DefinitionCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(created.kind, ClientKind::Sync);
    let DefinitionReply::Assistant(assistant) = created.reply else {
        panic!("a create call returns an assistant");
    };
    assert_eq!(serde_json::to_value(&assistant).unwrap(), parse(ASSISTANT));
    assert_eq!(
        sync_transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(CREATE_BODY),
            Auth::Key(KEY)
        )]
    );
    let (async_api, async_transport) = harness(200, ASSISTANT);
    let awaited = async_api
        .call(&request, None, Some(true), DefinitionCall::Create(&data))
        .await
        .unwrap();
    assert_eq!(awaited.kind, ClientKind::Async);
    assert_eq!(
        async_transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(CREATE_BODY),
            Auth::Key(KEY)
        )]
    );
    let nulls: AssistantCreate = serde_json::from_str(NULL_BODY).unwrap();
    let (null_api, null_transport) = harness(200, ASSISTANT);
    null_api
        .call(&request, None, None, DefinitionCall::Create(&nulls))
        .await
        .unwrap();
    assert_eq!(
        null_transport.requests(),
        vec![wire(
            "POST",
            ASSISTANTS_URL,
            Some(NULL_BODY),
            Auth::Key(KEY)
        )]
    );
}
