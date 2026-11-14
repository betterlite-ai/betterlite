use betterlite_models::decisions::Json;
use rstest::rstest;
use uuid::Uuid;

use crate::agent_identity::managed_write::{
    IdentityUpdate, ManagedWriteInput, managed_write_fields,
};
use crate::agent_identity::write_fields::{
    IdentityFields, IdentityHistoryWrite, IdentityRelationWrite, ManagedWriteFields,
};
use crate::agent_policy::{AgentExecutionMode, AgentResponse};
use crate::identity::{AgentIdentityFailure, FailureCode};

const TENANT: &str = "550e8400-e29b-41d4-a716-446655440000";
const CLIENT: &str = "550e8400-e29b-41d4-a716-446655440001";
const PRINCIPAL: &str = "550e8400-e29b-41d4-a716-446655440002";
const ISSUER: &str = "https://login.microsoftonline.com/550e8400-e29b-41d4-a716-446655440000/v2.0";
const AUTONOMOUS_PRINCIPAL_FAILURE: &str =
    "Autonomous mode requires the Enterprise application service-principal object ID";
const ENABLE_WITHOUT_BINDING_FAILURE: &str = "Bind an identity before enabling this managed agent";

const FIXED_REVISION: &str = "550e8400-e29b-41d4-a716-446655440010";
const OTHER_REVISION: &str = "550e8400-e29b-41d4-a716-446655440011";

fn fixed_revision() -> Uuid {
    Uuid::parse_str(FIXED_REVISION).unwrap()
}

fn other_revision() -> Uuid {
    Uuid::parse_str(OTHER_REVISION).unwrap()
}

fn write(
    input: &ManagedWriteInput,
    existing: Option<&AgentResponse>,
) -> Result<ManagedWriteFields, AgentIdentityFailure> {
    managed_write_fields(input, existing, fixed_revision)
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[derive(Clone, Copy)]
enum BindingState {
    Active,
    Inactive,
    WithoutPrincipal,
}

fn config_json(principal: Option<&str>) -> String {
    let principal_field = principal.map_or_else(String::new, |value| {
        format!(r#","service_principal_id":"{value}""#)
    });
    format!(
        r#"{{"provider":"microsoft_entra","tenant_id":"{TENANT}","client_id":"{CLIENT}"{principal_field}}}"#
    )
}

fn binding_json(state: BindingState, client: &str, revision: &str) -> String {
    let (active, principal_value) = match state {
        BindingState::Active => (true, format!("\"{PRINCIPAL}\"")),
        BindingState::Inactive => (false, format!("\"{PRINCIPAL}\"")),
        BindingState::WithoutPrincipal => (true, String::from("null")),
    };
    format!(
        r#"{{"agent_id":"agent","active":{active},"provider":"microsoft_entra","tenant_id":"{TENANT}","client_id":"{client}","service_principal_id":{principal_value},"issuer":"{ISSUER}","required_roles":[],"required_scopes":["user_impersonation"],"revision":"{revision}"}}"#
    )
}

fn agent(identity: Option<&str>, identity_managed: bool, execution_mode: &str) -> AgentResponse {
    let identity_json = identity.unwrap_or("null");
    serde_json::from_str(&format!(
        r#"{{"agent_id":"agent","agent_name":"agent","agent_card_params":{{}},"identity":{identity_json},"identity_managed":{identity_managed},"execution_mode":"{execution_mode}"}}"#
    ))
    .unwrap()
}

fn input(
    identity: IdentityUpdate,
    mode: Option<AgentExecutionMode>,
    enabled: Option<bool>,
) -> ManagedWriteInput {
    ManagedWriteInput {
        identity,
        execution_mode: mode,
        enabled,
    }
}

fn created_binding(fields: &ManagedWriteFields) -> &IdentityFields {
    match fields.identity.as_ref() {
        Some(IdentityRelationWrite::Create(binding)) => binding,
        other => panic!("expected a create write, got {other:?}"),
    }
}

#[test]
fn managed_write_fields_absent_identity_keeps_binding() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(&input(IdentityUpdate::Absent, None, None), Some(&existing)).unwrap();
    assert_eq!(fields, ManagedWriteFields::default());
}

#[test]
fn managed_write_fields_null_identity_unbinds() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(&input(IdentityUpdate::Null, None, None), Some(&existing)).unwrap();
    assert_eq!(fields.enabled, Some(false));
    assert_eq!(fields.identity_managed, Some(true));
    assert_eq!(fields.execution_mode, None);
    assert_eq!(fields.retired_identities, None);
    let Some(IdentityRelationWrite::Update(update)) = fields.identity.as_ref() else {
        panic!("expected an unbind update");
    };
    assert_eq!(update.active, Some(false));
    assert_eq!(update.last_authenticated_at, Some(None));
    assert_eq!(update.provider, None);
    assert_eq!(update.tenant_id, None);
    assert_eq!(update.revision.as_deref(), Some(FIXED_REVISION));
}

fn assert_archived_binding(binding: &IdentityFields) {
    assert_eq!(binding.provider.as_deref(), Some("microsoft_entra"));
    assert_eq!(binding.tenant_id.as_deref(), Some(TENANT));
    assert_eq!(binding.client_id.as_deref(), Some(CLIENT));
    assert_eq!(binding.issuer.as_deref(), Some(ISSUER));
    assert_eq!(
        binding.service_principal_id,
        Some(Some(String::from(PRINCIPAL)))
    );
    assert_eq!(binding.required_roles, Some(Vec::new()));
    assert_eq!(
        binding.required_scopes,
        Some(vec![String::from("user_impersonation")])
    );
    assert_eq!(binding.active, Some(true));
    assert_eq!(binding.last_authenticated_at, Some(None));
    assert_eq!(binding.revision.as_deref(), Some(FIXED_REVISION));
}

fn assert_retired_identity(history: &IdentityHistoryWrite) {
    assert_eq!(history.create.key.provider, "microsoft_entra");
    assert_eq!(history.create.key.tenant_id, TENANT);
    assert_eq!(history.create.key.client_id, CLIENT);
    assert_eq!(history.create.issuer, ISSUER);
}

#[test]
fn managed_write_fields_new_identity_archives_binding() {
    let fields = write(
        &input(
            IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
            None,
            None,
        ),
        None,
    )
    .unwrap();
    assert_eq!(fields.identity_managed, Some(true));
    assert_eq!(fields.enabled, None);
    assert_eq!(fields.execution_mode, None);
    assert_archived_binding(created_binding(&fields));
    assert_retired_identity(&fields.retired_identities.unwrap());
}

#[test]
fn managed_write_fields_existing_agent_upserts_binding() {
    let existing = agent(None, false, "autonomous");
    let fields = write(
        &input(
            IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
            None,
            None,
        ),
        Some(&existing),
    )
    .unwrap();
    let Some(IdentityRelationWrite::Upsert(upsert)) = fields.identity.as_ref() else {
        panic!("expected an upsert write");
    };
    assert_eq!(upsert.create, upsert.update);
    assert_eq!(upsert.create.active, Some(true));
    assert_eq!(
        upsert.create.service_principal_id,
        Some(Some(String::from(PRINCIPAL)))
    );
    assert!(fields.retired_identities.is_some());
}

#[test]
fn managed_write_fields_unchanged_identity_is_empty() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(
        &input(
            IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
            None,
            None,
        ),
        Some(&existing),
    )
    .unwrap();
    assert_eq!(fields, ManagedWriteFields::default());
}

#[test]
fn managed_write_fields_changed_client_rebinds() {
    let existing = agent(
        Some(&binding_json(
            BindingState::Active,
            "another-client",
            "rev-1",
        )),
        true,
        "autonomous",
    );
    let fields = write(
        &input(
            IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
            None,
            None,
        ),
        Some(&existing),
    )
    .unwrap();
    assert!(matches!(
        fields.identity,
        Some(IdentityRelationWrite::Upsert(_))
    ));
    assert!(fields.retired_identities.is_some());
}

#[test]
fn managed_write_fields_inactive_binding_rebinds() {
    let existing = agent(
        Some(&binding_json(BindingState::Inactive, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(
        &input(
            IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
            None,
            None,
        ),
        Some(&existing),
    )
    .unwrap();
    assert!(matches!(
        fields.identity,
        Some(IdentityRelationWrite::Upsert(_))
    ));
    assert_eq!(
        fields.retired_identities.unwrap().create.key.client_id,
        CLIENT
    );
}

#[rstest]
#[case::default_mode(None, true)]
#[case::autonomous(Some(AgentExecutionMode::Autonomous), true)]
#[case::both(Some(AgentExecutionMode::Both), true)]
#[case::delegated(Some(AgentExecutionMode::Delegated), false)]
fn managed_write_fields_autonomous_needs_service_principal(
    #[case] mode: Option<AgentExecutionMode>,
    #[case] fails: bool,
) {
    let result = write(
        &input(IdentityUpdate::Value(json(&config_json(None))), mode, None),
        None,
    );
    if fails {
        let failure = result.unwrap_err();
        assert_eq!(failure.message, AUTONOMOUS_PRINCIPAL_FAILURE);
        assert!(matches!(failure.code, FailureCode::IdentityDenied));
    } else {
        result.unwrap();
    }
}

#[rstest]
#[case::missing_binding(None, true)]
#[case::inactive_binding(Some(BindingState::Inactive), true)]
#[case::active_binding(Some(BindingState::Active), false)]
fn managed_write_fields_enabling_needs_active_binding(
    #[case] state: Option<BindingState>,
    #[case] fails: bool,
) {
    let identity = state.map(|value| binding_json(value, CLIENT, "rev-1"));
    let existing = agent(identity.as_deref(), true, "autonomous");
    let result = write(
        &input(IdentityUpdate::Absent, None, Some(true)),
        Some(&existing),
    );
    if fails {
        assert_eq!(result.unwrap_err().message, ENABLE_WITHOUT_BINDING_FAILURE);
    } else {
        result.unwrap();
    }
}

#[test]
fn managed_write_fields_enabling_with_null_identity_unbinds() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(
        &input(IdentityUpdate::Null, None, Some(true)),
        Some(&existing),
    )
    .unwrap();
    assert_eq!(fields.enabled, Some(false));
}

#[test]
fn managed_write_fields_enabled_false_is_written() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(
        &input(IdentityUpdate::Absent, None, Some(false)),
        Some(&existing),
    )
    .unwrap();
    assert_eq!(fields.enabled, Some(false));
    assert_eq!(fields.identity, None);
}

#[test]
fn managed_write_fields_absent_identity_without_binding_is_allowed() {
    let existing = agent(None, true, "autonomous");
    let fields = write(&input(IdentityUpdate::Absent, None, None), Some(&existing)).unwrap();
    assert_eq!(fields, ManagedWriteFields::default());
}

#[test]
fn managed_write_fields_disabling_without_active_binding_is_allowed() {
    let existing = agent(
        Some(&binding_json(BindingState::Inactive, CLIENT, "rev-1")),
        true,
        "autonomous",
    );
    let fields = write(
        &input(IdentityUpdate::Absent, None, Some(false)),
        Some(&existing),
    )
    .unwrap();
    assert_eq!(fields.enabled, Some(false));
    assert_eq!(fields.identity, None);
}

#[test]
fn managed_write_fields_existing_binding_without_principal_fails() {
    let existing = agent(
        Some(&binding_json(
            BindingState::WithoutPrincipal,
            CLIENT,
            "rev-1",
        )),
        true,
        "autonomous",
    );
    let failure = write(&input(IdentityUpdate::Absent, None, None), Some(&existing)).unwrap_err();
    assert_eq!(failure.message, AUTONOMOUS_PRINCIPAL_FAILURE);
}

#[test]
fn managed_write_fields_mode_defaults_to_existing() {
    let existing = agent(
        Some(&binding_json(
            BindingState::WithoutPrincipal,
            CLIENT,
            "rev-1",
        )),
        true,
        "delegated",
    );
    let fields = write(&input(IdentityUpdate::Absent, None, None), Some(&existing)).unwrap();
    assert_eq!(fields.execution_mode, None);
}

#[test]
fn managed_write_fields_mode_from_request_is_written() {
    let existing = agent(
        Some(&binding_json(
            BindingState::WithoutPrincipal,
            CLIENT,
            "rev-1",
        )),
        true,
        "autonomous",
    );
    let fields = write(
        &input(
            IdentityUpdate::Absent,
            Some(AgentExecutionMode::Delegated),
            None,
        ),
        Some(&existing),
    )
    .unwrap();
    assert_eq!(fields.execution_mode, Some(AgentExecutionMode::Delegated));
}

#[rstest]
#[case::invalid_uuid(
    r#"{"provider":"microsoft_entra","tenant_id":"nope","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#
)]
#[case::unknown_field(
    r#"{"provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","extra":true}"#
)]
#[case::wrong_provider(
    r#"{"provider":"google","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001"}"#
)]
fn managed_write_fields_invalid_identity_is_failure(#[case] identity: &str) {
    let failure = write(
        &input(IdentityUpdate::Value(json(identity)), None, None),
        None,
    )
    .unwrap_err();
    assert!(
        failure
            .message
            .starts_with("Invalid agent identity configuration: ")
    );
    assert!(matches!(failure.code, FailureCode::IdentityDenied));
}

#[test]
fn managed_write_fields_uses_injected_revision() {
    let write_input = input(
        IdentityUpdate::Value(json(&config_json(Some(PRINCIPAL)))),
        None,
        None,
    );
    let first = managed_write_fields(&write_input, None, fixed_revision).unwrap();
    let second = managed_write_fields(&write_input, None, other_revision).unwrap();
    assert_eq!(
        created_binding(&first).revision.as_deref(),
        Some(FIXED_REVISION)
    );
    assert_eq!(
        created_binding(&second).revision.as_deref(),
        Some(OTHER_REVISION)
    );
}

#[test]
fn managed_write_fields_unbind_without_managed_flag_keeps_enabled() {
    let existing = agent(
        Some(&binding_json(BindingState::Active, CLIENT, "rev-1")),
        false,
        "autonomous",
    );
    let fields = write(&input(IdentityUpdate::Null, None, None), Some(&existing)).unwrap();
    assert_eq!(fields.enabled, None);
    assert_eq!(fields.identity_managed, None);
    assert!(matches!(
        fields.identity,
        Some(IdentityRelationWrite::Update(_))
    ));
}

#[test]
fn managed_write_fields_unbind_without_identity_disables_agent() {
    let existing = agent(None, true, "autonomous");
    let fields = write(&input(IdentityUpdate::Null, None, None), Some(&existing)).unwrap();
    assert_eq!(fields.identity, None);
    assert_eq!(fields.identity_managed, Some(true));
    assert_eq!(fields.enabled, Some(false));
}

#[test]
fn managed_write_fields_null_identity_without_agent_is_empty() {
    let fields = write(&input(IdentityUpdate::Null, None, None), None).unwrap();
    assert_eq!(fields, ManagedWriteFields::default());
}
