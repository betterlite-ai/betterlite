use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, map_role};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::agents::collection_routes::guards::{
    CallerKeyMissing, enforce_provider_key,
};
use betterlite_server::api::agents::collection_routes::merge::{
    merge_query_params_into_data, query_values,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IsAdmin {
        user_role: Option<String>,
    },
    EnforceKey {
        user_role: Option<String>,
        data: Map<String, Value>,
    },
    MergeQuery {
        data: Map<String, Value>,
        query: String,
    },
}

// the auth stage's role mapping is the same one map_role applies to a LiteLLM user_role
fn caller(user_role: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: "user-1".to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: map_role(user_role).into_iter().collect(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

fn enforce_key(user_role: Option<&str>, data: &Map<String, Value>) -> Value {
    match enforce_provider_key(data, &caller(user_role)) {
        Ok(()) => json!({"ok": true}),
        Err(_missing) => json!({
            "ok": false,
            "status": CallerKeyMissing::STATUS.as_u16(),
            "detail": CallerKeyMissing::DETAIL,
        }),
    }
}

fn merge_query(data: &Map<String, Value>, query: &str) -> Value {
    let mut merged = data.clone();
    merge_query_params_into_data(&mut merged, &query_values(Some(query)));
    Value::Object(merged)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answer = match input {
            Input::IsAdmin { user_role } => Value::Bool(
                enforce_provider_key(&Map::new(), &caller(user_role.as_deref())).is_ok(),
            ),
            Input::EnforceKey { user_role, data } => enforce_key(user_role.as_deref(), data),
            Input::MergeQuery { data, query } => merge_query(data, query),
        };
        Ok(answer)
    }
}
