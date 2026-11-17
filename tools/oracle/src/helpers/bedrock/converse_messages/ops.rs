use betterlite_bedrock::converse_messages::error::ConverseMessageError;
use betterlite_bedrock::converse_messages::guarded::guard_trailing_user_turns;
use betterlite_bedrock::converse_messages::orphans::{OrphanPolicy, handle_orphaned_tool_blocks};
use betterlite_bedrock::converse_messages::system::transform_system_message;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::input::{GuardedInput, HandleOrphanedInput, Input, TransformSystemInput};

const MESSAGE_POLICY: &str = "ConverseMessageError";
const UNSUPPORTED_PARAMS: &str = "UnsupportedParamsError";

fn helper_error(error: &ConverseMessageError) -> Box<HelperError> {
    let (class, message) = match error {
        ConverseMessageError::ToolCallsWithoutTools => (
            UNSUPPORTED_PARAMS,
            format!("litellm.UnsupportedParamsError: {error}"),
        ),
        ConverseMessageError::TextPartWithoutText { .. } | ConverseMessageError::Params(_) => {
            (MESSAGE_POLICY, error.to_string())
        }
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn guarded(call: &GuardedInput) -> HelperResult {
    let messages = guard_trailing_user_turns(&call.messages, &call.optional_params)
        .map_err(|error| helper_error(&error))?;
    Ok(json!({ "messages": messages }))
}

fn handle_orphaned(call: &HandleOrphanedInput) -> HelperResult {
    let mut optional_params = call.optional_params.clone();
    let messages = handle_orphaned_tool_blocks(
        &call.messages,
        &mut optional_params,
        OrphanPolicy {
            neutralize: call.neutralize_orphaned_blocks,
            modify_params: call.modify_params,
        },
    )
    .map_err(|error| helper_error(&error))?;
    Ok(json!({ "messages": messages, "optional_params": optional_params }))
}

fn transform_system(call: &TransformSystemInput) -> HelperResult {
    let (messages, system) =
        transform_system_message(&call.messages, call.model.as_deref(), &call.model_cost)
            .map_err(|error| helper_error(&error))?;
    Ok(json!({ "messages": messages, "system": system }))
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Guarded(call) => guarded(call),
        Input::HandleOrphaned(call) => handle_orphaned(call),
        Input::TransformSystem(call) => transform_system(call),
    }
}
