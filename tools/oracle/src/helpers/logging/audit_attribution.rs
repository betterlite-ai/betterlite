use betterlite_config::sections::settings::JsonValue;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::audit_events::identity::{CallerIdentity, get_audit_log_changed_by};
use betterlite_server::audit_events::payload::build_audit_log_payload;
use betterlite_server::audit_events::record::decode_audit_record;
use serde::Deserialize;
use serde_json::json;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Identity(IdentityInput),
    Payload(PayloadInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityInput {
    litellm_changed_by: Option<String>,
    user_id: Option<String>,
    admin_name: Option<String>,
    #[serde(default)]
    metadata: Option<JsonValue>,
    #[serde(default)]
    team_metadata: Option<JsonValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PayloadInput {
    id: String,
    updated_at: String,
    #[serde(default)]
    changed_by: Option<String>,
    #[serde(default)]
    changed_by_api_key: Option<String>,
    action: String,
    table_name: String,
    object_id: String,
    #[serde(default)]
    before_value: Option<JsonValue>,
    #[serde(default)]
    updated_values: Option<JsonValue>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Identity(case) => identity(case),
            Input::Payload(case) => payload(case),
        }
    }
}

// get_audit_log_changed_by: the authenticated user wins unless the key's metadata grants the header
fn identity(case: &IdentityInput) -> HelperResult {
    let caller = CallerIdentity {
        user_id: case.user_id.clone(),
        api_key: None,
        metadata: case.metadata.clone(),
        team_metadata: case.team_metadata.clone(),
    };
    let changed_by = get_audit_log_changed_by(
        case.litellm_changed_by.as_deref(),
        &caller,
        case.admin_name.as_deref(),
    );
    Ok(json!({ "changed_by": changed_by }))
}

// A LiteLLM_AuditLogs model, validated and masked by the record constructor, then read as the callback payload
fn payload(case: &PayloadInput) -> HelperResult {
    let document = json!({
        "id": case.id,
        "updated_at": case.updated_at,
        "changed_by": case.changed_by,
        "changed_by_api_key": case.changed_by_api_key,
        "action": case.action,
        "table_name": case.table_name,
        "object_id": case.object_id,
        "before_value": case.before_value,
        "updated_values": case.updated_values,
    });
    let record = decode_audit_record(document.to_string().as_bytes()).or_value_error()?;
    let field = build_audit_log_payload(&record);
    Ok(json!({
        "id": field.id,
        "updated_at": field.updated_at,
        "changed_by": field.changed_by,
        "changed_by_api_key": field.hashed_changed_by_api_key,
        "action": field.action,
        "table_name": field.table_name,
        "object_id": field.object_id,
        "before_value": record.before_value,
        "updated_values": record.updated_values,
    }))
}
