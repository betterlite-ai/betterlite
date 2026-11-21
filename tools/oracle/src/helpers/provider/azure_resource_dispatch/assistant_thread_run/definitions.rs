use std::sync::Arc;

use betterlite_azure::assistants::error::AssistantsError;
use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi, DefinitionCall, DefinitionReply,
};
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_provider::azure_resource_dispatch::assistants::{self, DefinitionsDispatch};
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::json;

use super::super::support::{AssistantsFake, OTHER, failure, failure_with_status, runtime};
use super::{Answer, Step};

const CREATE_TEXT: &str = r#"{"model":"gpt-4o"}"#;

// B686's three definition facades, each answered by the one fake transport
pub(super) fn answer(step: &Step) -> Answer {
    let reply = match step {
        Step::DefinitionsCreate(value)
        | Step::DefinitionsList(value)
        | Step::DefinitionsDelete(value) => value,
        _ => return Err(failure(OTHER)),
    };
    let runtime = runtime()?;
    let fake = Arc::new(AssistantsFake::new(vec![reply.planned()]));
    let fixture = reply.fixture();
    let request = fixture.request();
    let api = AssistantsApi {
        transport: Arc::clone(&fake),
    };
    let data: AssistantCreate = serde_json::from_str(CREATE_TEXT).map_err(failure)?;
    let (kind, call) = match step {
        Step::DefinitionsCreate(_) => ("definitions_create", DefinitionCall::Create(&data)),
        Step::DefinitionsList(_) => ("definitions_list", DefinitionCall::List),
        Step::DefinitionsDelete(_) => (
            "definitions_delete",
            DefinitionCall::Delete(&reply.target_id),
        ),
        _ => return Err(failure(OTHER)),
    };
    let result = runtime.block_on(dispatch(AzureResourceOperation::AssistantDefinitions(
        assistants::definitions(DefinitionsDispatch {
            api: &api,
            request: &request,
            supplied: None,
            flag: None,
            call,
        }),
    )));
    let value = match result {
        Ok(AzureResourceResult::AssistantDefinitions(reply)) => match reply.reply {
            DefinitionReply::Page(page) => json!({"count": page.data.len()}),
            DefinitionReply::Assistant(assistant) => json!({"id": assistant.id}),
            DefinitionReply::Deleted(deleted) => json!({"deleted": deleted.deleted}),
        },
        Ok(_) => return Err(failure(OTHER)),
        Err(AzureResourceError::AssistantDefinitions(DefinitionsError::Assistants(
            AssistantsError::Status { status, .. },
        ))) => return Err(failure_with_status(status, "the fixture failed")),
        Err(error) => return Err(failure(error)),
    };
    Ok((kind, value))
}
