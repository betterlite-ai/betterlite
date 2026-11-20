mod validate;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::team_fields::schema::parse_team_metadata_schema;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::team_fields::rows::{
    CallerRole, GeneralSettingsMap, LogValue, TeamEditAccess, caller_edit_access, sanitize_for_log,
};
use betterlite_server::team_fields::validation::{
    TeamMetadataValidator, TeamMetadataValidatorRegistry,
};
use serde::Deserialize;
use serde_json::{Value, json};

use self::validate::{Behaviour, FakeValidator, validate_answer};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequesterInput {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_email: Option<String>,
    #[serde(default)]
    user_role: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Schema {
        schema: JsonValue,
    },
    Registry {
        steps: Vec<String>,
    },
    Validate {
        general_settings: BTreeMap<String, JsonValue>,
        validator: String,
        operation: String,
        #[serde(default)]
        metadata: Option<JsonValue>,
        #[serde(default)]
        existing_metadata: Option<JsonValue>,
        #[serde(default)]
        team_id: Option<String>,
        #[serde(default)]
        team_alias: Option<String>,
        requester: RequesterInput,
    },
    Sanitize {
        value: String,
    },
    EditAccess {
        #[serde(default)]
        role: Option<String>,
        general_settings: BTreeMap<String, JsonValue>,
    },
}

struct ValidateCall<'input> {
    general_settings: &'input BTreeMap<String, JsonValue>,
    validator: &'input str,
    operation: &'input str,
    metadata: Option<&'input JsonValue>,
    existing_metadata: Option<&'input JsonValue>,
    team_id: Option<&'input str>,
    team_alias: Option<&'input str>,
    requester: &'input RequesterInput,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Schema { schema } => Ok(schema_answer(schema)),
            Input::Registry { steps } => registry_answer(steps),
            Input::Validate {
                general_settings,
                validator,
                operation,
                metadata,
                existing_metadata,
                team_id,
                team_alias,
                requester,
            } => validate_answer(&ValidateCall {
                general_settings,
                validator,
                operation,
                metadata: metadata.as_ref(),
                existing_metadata: existing_metadata.as_ref(),
                team_id: team_id.as_deref(),
                team_alias: team_alias.as_deref(),
                requester,
            }),
            Input::Sanitize { value } => Ok(json!({
                "text": sanitize_for_log(LogValue::Display(value)),
            })),
            Input::EditAccess {
                role,
                general_settings,
            } => edit_access_answer(role.as_deref(), general_settings),
        }
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn schema_answer(schema: &JsonValue) -> Value {
    match parse_team_metadata_schema(Some(schema)) {
        Ok(fields) => json!({
            "fields": fields
                .iter()
                .map(|field| json!({"key": field.key, "label": field.label}))
                .collect::<Vec<_>>(),
        }),
        Err(_) => json!({"failed": true}),
    }
}

fn registry_answer(steps: &[String]) -> HelperResult {
    let registry = TeamMetadataValidatorRegistry::default();
    let first: Arc<dyn TeamMetadataValidator> =
        Arc::new(FakeValidator::new(Behaviour::Accept, true));
    let second: Arc<dyn TeamMetadataValidator> =
        Arc::new(FakeValidator::new(Behaviour::Reject, true));
    let mut seen = Vec::new();
    for step in steps {
        match step.as_str() {
            "get" => {
                let current = registry.get();
                let name = match current {
                    None => "none",
                    Some(found) if Arc::ptr_eq(&found, &first) => "a",
                    Some(_) => "b",
                };
                seen.push(name);
            }
            "set_a" => registry.set(Some(Arc::clone(&first))),
            "set_b" => registry.set(Some(Arc::clone(&second))),
            "clear" => registry.set(None),
            other => return Err(invalid(format!("unknown registry step {other}"))),
        }
    }
    Ok(json!({"seen": seen}))
}

fn edit_access_answer(
    role: Option<&str>,
    general_settings: &BTreeMap<String, JsonValue>,
) -> HelperResult {
    let caller = match role {
        None => None,
        Some("proxy_admin") => Some(CallerRole::ProxyAdmin),
        Some("org_admin") => Some(CallerRole::OrgAdmin),
        Some("team_admin") => Some(CallerRole::TeamAdmin),
        Some(other) => return Err(invalid(format!("unknown role {other}"))),
    };
    let settings = GeneralSettingsMap {
        entries: general_settings.clone(),
    };
    let (kind, fields) = match caller_edit_access(caller, &settings) {
        TeamEditAccess::Unrestricted => ("unrestricted", None),
        TeamEditAccess::TeamAdmin { editable_fields } => ("team_admin", Some(editable_fields)),
        TeamEditAccess::TeamAdminDisabled => ("team_admin_disabled", None),
        TeamEditAccess::None => ("none", None),
    };
    Ok(json!({"kind": kind, "editable_fields": fields}))
}
