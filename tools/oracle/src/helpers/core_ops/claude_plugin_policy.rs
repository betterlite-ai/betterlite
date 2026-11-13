use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, map_role};
use betterlite_core::api::claude_plugin_policy::source::{PluginSource, validate_plugin_source};
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, PluginCaller, PluginObjectPermission, PluginRecord, require_database,
    require_proxy_admin, skill_visibility,
};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    ValidateSource {
        source: PluginSource,
    },
    RequireAdmin {
        caller: CallerFields,
    },
    Visibility {
        caller: Option<CallerFields>,
        plugins: Vec<PluginRecord>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerFields {
    #[serde(default)]
    user_role: Option<String>,
    #[serde(default)]
    key_skills: Option<Vec<String>>,
    #[serde(default)]
    team_skills: Option<Vec<String>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call));
        }
        Ok(json!({ "calls": answers }))
    }
}

fn answer(call: &Call) -> Value {
    match call {
        Call::ValidateSource { source } => match validate_plugin_source(source) {
            Ok(()) => json!({ "ok": true, "status": null, "value": null }),
            Err(error) => rejection(&error),
        },
        Call::RequireAdmin { caller } => require_admin_answer(caller),
        Call::Visibility { caller, plugins } => visibility_answer(caller.as_ref(), plugins),
    }
}

fn rejection(error: &ClaudePluginPolicyError) -> Value {
    json!({
        "ok": false,
        "status": error.fields().status_code,
        "value": null,
    })
}

fn require_admin_answer(fields: &CallerFields) -> Value {
    let caller = plugin_caller(fields);
    let guard = require_proxy_admin(Some(&caller)).and_then(|()| require_database(false));
    match guard {
        Ok(()) => json!({ "ok": true, "status": null, "value": null }),
        Err(error) => rejection(&error),
    }
}

fn visibility_answer(caller: Option<&CallerFields>, plugins: &[PluginRecord]) -> Value {
    let resolved = caller.map(plugin_caller);
    let visibility = skill_visibility(resolved.as_ref());
    let allowed: Vec<&str> = plugins
        .iter()
        .filter(|plugin| visibility.allows(plugin))
        .map(|plugin| plugin.name.as_str())
        .collect();
    json!({
        "ok": true,
        "status": null,
        "value": {
            "granted": visibility.granted.iter().collect::<Vec<&String>>(),
            "sees_private": visibility.sees_private,
            "allowed": allowed,
        },
    })
}

fn plugin_caller(fields: &CallerFields) -> PluginCaller {
    PluginCaller {
        principal: principal(fields.user_role.as_deref()),
        object_permission: permission(fields.key_skills.as_deref()),
        team_object_permission: permission(fields.team_skills.as_deref()),
    }
}

fn permission(values: Option<&[String]>) -> Option<PluginObjectPermission> {
    let values = values?;
    Some(PluginObjectPermission {
        skills: Some(values.to_vec()),
    })
}

fn principal(role: Option<&str>) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: String::new(),
        issuer: None,
        audience: Vec::new(),
        user: None,
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: map_role(role).into_iter().collect(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}
