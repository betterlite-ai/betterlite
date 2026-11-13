use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::mcp_tool_hook::types::{McpPostCallResponse, McpToolHookCall};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::input::{Behavior, LoggerWire};

pub(super) struct ScriptedLogger {
    pub(super) wire: LoggerWire,
}

fn edited_item() -> Json {
    Json::Object(IndexMap::from([
        ("type".to_owned(), Json::Text("text".to_owned())),
        ("text".to_owned(), Json::Text("edited".to_owned())),
    ]))
}

impl ScriptedLogger {
    fn rewrite(&self, call: &mut McpToolHookCall<'_>) {
        call.original.content.clone_from(&self.wire.content);
        call.original
            .structured_content
            .clone_from(&self.wire.structured_content);
    }

    fn fail(&self) -> Result<Option<McpPostCallResponse>, LoggerError> {
        Err(LoggerError::Delivery {
            callback: self.wire.name.clone(),
            message: "logger exploded".to_owned(),
        })
    }

    fn replace(&self, call: &McpToolHookCall<'_>) -> McpPostCallResponse {
        let mut hidden_params = call.response.hidden_params;
        if let Some(cost) = self.wire.response_cost {
            hidden_params.response_cost = Some(cost);
        }
        McpPostCallResponse {
            mcp_tool_call_response: self.wire.content.clone(),
            hidden_params,
        }
    }
}

#[async_trait]
impl CallbackLogger for ScriptedLogger {
    fn callback_name(&self) -> &str {
        &self.wire.name
    }

    async fn async_post_mcp_tool_call_hook(
        &self,
        mut call: McpToolHookCall<'_>,
    ) -> Result<Option<McpPostCallResponse>, LoggerError> {
        match self.wire.behavior {
            Behavior::None => Ok(None),
            Behavior::EditInPlace => {
                call.response.mcp_tool_call_response.push(edited_item());
                Ok(None)
            }
            Behavior::Replace => Ok(Some(self.replace(&call))),
            Behavior::ReplaceUnchanged => Ok(Some(call.response.clone())),
            Behavior::Fail => self.fail(),
            Behavior::RewriteLive => {
                self.rewrite(&mut call);
                Ok(None)
            }
            Behavior::RewriteLiveThenFail => {
                self.rewrite(&mut call);
                self.fail()
            }
        }
    }
}
