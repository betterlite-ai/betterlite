use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_server::guardrail_message_scope::selection::error::SelectionError;
use betterlite_server::guardrail_message_scope::selection::{
    CallbackNames, can_modify_guardrails, guardrail_applies,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Team {
    metadata: Option<Object>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanModify {
    team: Option<Team>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Proceed {
    data: Object,
    guardrail_name: String,
    callbacks: CallbackNames,
}

pub(super) fn can_modify(request: &CanModify) -> Value {
    let metadata = request
        .team
        .as_ref()
        .and_then(|team| team.metadata.as_ref());
    json!({"can_modify": can_modify_guardrails(metadata)})
}

pub(super) fn proceed(request: &Proceed) -> Value {
    match guardrail_applies(&request.data, &request.guardrail_name, &request.callbacks) {
        Ok(applies) => json!({"proceed": applies}),
        Err(SelectionError::UnknownGuardrail { guardrail_name }) => {
            json!({"unknown_guardrail": guardrail_name})
        }
    }
}
