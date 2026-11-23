use std::sync::Arc;

use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_azure::azure_assistant_definitions::operations::{
    AssistantsApi, DefinitionCall, DefinitionReply,
};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::records::AssistantCreate;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::azure_resource_dispatch::assistants::DefinitionsDispatch;
use betterlite_provider::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};
use serde_json::{Value, json};

use super::super::support::{
    AssistantsFake, OTHER, Reply as FakeReply, failure, runtime, with_status,
};
use super::{Answer, Fixture, Reply, Step};

const CREATE_TEXT: &str = r#"{"model":"gpt-4o"}"#;

// B686's three definition facades, each answered by the one fake transport
pub(super) fn answer(step: &Step) -> Answer {
    let (Step::DefinitionsCreate(reply)
    | Step::DefinitionsList(reply)
    | Step::DefinitionsDelete(reply)) = step
    else {
        return Err(failure(OTHER));
    };
    let runtime = runtime()?;
    let fake = Arc::new(AssistantsFake::new(vec![FakeReply::json(
        reply.status,
        &reply.body,
    )]));
    let fixture = Fixture::new(&reply.api_key, &reply.api_base, &reply.api_version);
    let request = fixture.request();
    let api = AssistantsApi {
        transport: Arc::clone(&fake),
    };
    let data: AssistantCreate = serde_json::from_str(CREATE_TEXT).map_err(failure)?;
    let (kind, call) = call_of(step, reply, &data)?;
    let result = runtime.block_on(dispatch(AzureResourceOperation::AssistantDefinitions(
        Box::pin(
            DefinitionsDispatch {
                api: &api,
                request: &request,
                supplied: None,
                flag: None,
                call,
            }
            .run(),
        ),
    )));
    Ok((kind, value_of(result)?))
}

fn call_of<'call>(
    step: &'call Step,
    reply: &'call Reply,
    data: &'call AssistantCreate,
) -> Result<(&'static str, DefinitionCall<'call>), Box<HelperError>> {
    match step {
        Step::DefinitionsCreate(_) => Ok(("definitions_create", DefinitionCall::Create(data))),
        Step::DefinitionsList(_) => Ok(("definitions_list", DefinitionCall::List)),
        Step::DefinitionsDelete(_) => Ok((
            "definitions_delete",
            DefinitionCall::Delete(&reply.target_id),
        )),
        Step::ThreadCreate(_)
        | Step::ThreadGet(_)
        | Step::MessageCreate(_)
        | Step::MessageList(_)
        | Step::RunPoll(_)
        | Step::RunStream(_) => Err(failure(OTHER)),
    }
}

fn value_of(
    result: Result<AzureResourceResult, AzureResourceError>,
) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(AzureResourceResult::AssistantDefinitions(reply)) => match reply.reply {
            DefinitionReply::Page(page) => Ok(json!({"count": page.data.len()})),
            DefinitionReply::Assistant(assistant) => Ok(json!({"id": assistant.id})),
            DefinitionReply::Deleted(deleted) => Ok(json!({"deleted": deleted.deleted})),
        },
        Ok(_) => Err(failure(OTHER)),
        Err(AzureResourceError::AssistantDefinitions(DefinitionsError::Assistants(
            AssistantsError::Status { status, .. },
        ))) => Err(with_status(failure("the fixture failed"), status)),
        Err(error) => Err(failure(error)),
    }
}
