use std::collections::BTreeMap;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::ExceptionCode;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

use crate::auth_model_lists::lists::NO_DEFAULT_MODELS;
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::{ModelAccessDecision, model_denied_client_message};
use crate::auth_model_patterns::grant::ModelGrantPattern;
use crate::auth_request_models::RequestedModels;
use crate::identity_grants::resolver::user_models;

mod tests;

#[must_use]
pub fn can_user_call_model(
    model: &RequestedModels,
    user: Option<&LiteLLMUserTable>,
    key_model_aliases: &BTreeMap<String, String>,
    context: &ModelAccessContext<'_>,
) -> ModelAccessDecision {
    let Some(user) = user else {
        return ModelAccessDecision::Allowed;
    };
    let models = user_models(user);
    if models.iter().any(|name| name.as_str() == NO_DEFAULT_MODELS) {
        let requested = requested_text(model);
        // LiteLLM raises the key type here on purpose, even though the object is a user
        return ModelAccessDecision::Denied(Box::new(ModelAccessDeniedProxyException::new(
            model_denied_client_message(&requested),
            format!(
                "User not allowed to access model. No default model access, only team models \
                 allowed. Tried to access {requested}"
            ),
            ProxyErrorTypes::KeyModelAccessDenied.to_string(),
            Some("model".to_owned()),
            ExceptionCode::Status(403),
        )));
    }
    can_object_call_model(
        model,
        &ModelGrantPattern {
            models,
            team_model_aliases: BTreeMap::new(),
            key_model_aliases: key_model_aliases.clone(),
            team_id: None,
        },
        ModelAccessObject::User,
        context,
        0,
    )
}

// Python's f-string prints a list through str(); the joined names match the depth message can_object_call_model writes
fn requested_text(model: &RequestedModels) -> String {
    match model {
        RequestedModels::None => String::new(),
        RequestedModels::One(name) => name.clone(),
        RequestedModels::Many(names) => names.join(","),
    }
}
