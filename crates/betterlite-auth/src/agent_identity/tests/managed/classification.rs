use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use rstest::rstest;

use crate::agent_identity::classification::classify_agent_subject;
use crate::agent_policy::{
    AgentExecutionMode, AgentIdentityBinding, IdentityProvider, PolicyFlag, RequiredScopes,
};
use crate::identity::{AgentIdentityFailure, AgentSubject, FailureCode, SubjectKind, SubjectMode};

const ISSUER: &str = "https://login.microsoftonline.com/tenant/v2.0";
const MISMATCH: &str = "Token does not match the registered Entra application";
const NO_OID: &str = "Entra token must identify its object subject";
const NATIVE_USER: &str = "Native agent-user authentication is not supported by this binding";
const INVALID_SCOPE: &str = "Invalid delegated scope claim";
const DELEGATED_CONTRADICTION: &str =
    "Delegated token contradicts the configured agent identity or mode";
const MISSING_SCOPES: &str = "Token lacks the required delegated scopes";
const APPLICATION_CONTRADICTION: &str =
    "Application token contradicts the configured agent identity or mode";
const MISSING_ROLES: &str = "Token lacks the required application roles";
const INVALID_ROLES: &str = "Invalid application roles claim";

fn text(value: &str) -> JsonData {
    JsonData::String(value.to_owned())
}

fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

fn list(values: Vec<JsonData>) -> JsonData {
    JsonData::Array(values)
}

fn claims(entries: &[(&str, JsonData)]) -> Payload {
    Payload(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn binding(scopes: &[&str], roles: &[&str], principal: Option<&str>) -> AgentIdentityBinding {
    AgentIdentityBinding {
        agent_id: String::from("agent"),
        active: PolicyFlag(true),
        provider: IdentityProvider::MicrosoftEntra,
        tenant_id: String::from("tenant"),
        client_id: String::from("client"),
        service_principal_id: principal.map(str::to_owned),
        issuer: String::from(ISSUER),
        required_roles: roles.iter().map(|role| (*role).to_owned()).collect(),
        required_scopes: RequiredScopes(scopes.iter().map(|scope| (*scope).to_owned()).collect()),
        revision: String::from("revision"),
        last_authenticated_at: None,
    }
}

fn application_claims() -> Payload {
    claims(&[
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("sp")),
    ])
}

fn delegated_claims(scope: &str) -> Payload {
    let mut token = application_claims();
    token.0.insert(String::from("oid"), text("user"));
    token.0.insert(String::from("scp"), text(scope));
    token
}

fn claims_with(oid: &str, idtyp: Option<&str>, scope: Option<&str>) -> Payload {
    let mut token = application_claims();
    token.0.insert(String::from("oid"), text(oid));
    if let Some(value) = scope {
        token.0.insert(String::from("scp"), text(value));
    }
    if let Some(claim) = idtyp {
        token.0.insert(String::from("idtyp"), text(claim));
    }
    token
}

fn failure_of(result: Result<AgentSubject, AgentIdentityFailure>) -> AgentIdentityFailure {
    match result {
        Ok(subject) => panic!("expected a failure, got {subject:?}"),
        Err(failure) => failure,
    }
}

fn subject_of(result: Result<AgentSubject, AgentIdentityFailure>) -> AgentSubject {
    match result {
        Ok(subject) => subject,
        Err(failure) => panic!("expected a subject, got {failure:?}"),
    }
}

#[rstest]
#[case::issuer("elsewhere", "tenant", "client")]
#[case::tenant(ISSUER, "other-tenant", "client")]
#[case::client(ISSUER, "tenant", "other-client")]
fn managed_classify_rejects_token_binding_mismatch(
    #[case] issuer: &str,
    #[case] tenant: &str,
    #[case] client: &str,
) {
    let identity = binding(&["user_impersonation"], &[], Some("sp"));
    let token = claims(&[
        ("iss", text(issuer)),
        ("tid", text(tenant)),
        ("azp", text(client)),
        ("oid", text("sp")),
    ]);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, MISMATCH);
    assert!(matches!(failure.code, FailureCode::IdentityDenied));
}

#[test]
fn managed_classify_rejects_non_text_issuer() {
    let identity = binding(&[], &[], Some("sp"));
    let token = claims(&[
        ("iss", number(5)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("sp")),
    ]);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, MISMATCH);
}

#[rstest]
#[case::missing(vec![])]
#[case::empty(vec![("oid", text(""))])]
#[case::number(vec![("oid", number(7))])]
#[case::null(vec![("oid", JsonData::Null)])]
fn managed_classify_requires_object_subject(#[case] oid: Vec<(&str, JsonData)>) {
    let identity = binding(&[], &[], Some("sp"));
    let mut entries = vec![
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
    ];
    entries.extend(oid);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &claims(&entries),
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, NO_OID);
}

#[rstest]
#[case::user_facet("13", false)]
#[case::user_facet_among_others("1 13", false)]
#[case::other_facet("1", true)]
fn managed_classify_checks_native_user_facet(#[case] facets: &str, #[case] accepted: bool) {
    let identity = binding(&[], &[], Some("sp"));
    let token = claims(&[
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("sp")),
        ("xms_sub_fct", text(facets)),
    ]);
    let result = classify_agent_subject(&identity, &token, &AgentExecutionMode::Autonomous);
    if accepted {
        assert_eq!(subject_of(result).kind, SubjectKind::Application);
    } else {
        assert_eq!(failure_of(result).message, NATIVE_USER);
    }
}

#[test]
fn managed_classify_rejects_non_text_facet() {
    let identity = binding(&[], &[], Some("sp"));
    let token = claims(&[
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("sp")),
        ("xms_sub_fct", number(13)),
    ]);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, NATIVE_USER);
}

#[rstest]
#[case::delegated(AgentExecutionMode::Delegated, true)]
#[case::delegated_both(AgentExecutionMode::Both, true)]
#[case::application(AgentExecutionMode::Autonomous, false)]
#[case::application_both(AgentExecutionMode::Both, false)]
fn managed_classify_subject_kind(#[case] mode: AgentExecutionMode, #[case] delegated: bool) {
    let identity = binding(&["user_impersonation"], &["admin"], Some("sp"));
    let token = if delegated {
        delegated_claims("other user_impersonation")
    } else {
        let mut application = application_claims();
        application
            .0
            .insert(String::from("roles"), list(vec![text("admin")]));
        application
    };
    let subject = subject_of(classify_agent_subject(&identity, &token, &mode));
    let (kind, subject_mode, oid) = if delegated {
        (
            SubjectKind::DelegatedSubject,
            SubjectMode::Delegated,
            "user",
        )
    } else {
        (SubjectKind::Application, SubjectMode::Autonomous, "sp")
    };
    assert_eq!(subject.kind, kind);
    assert_eq!(subject.mode, subject_mode);
    assert_eq!(subject.oid, oid);
}

#[test]
fn managed_classify_delegated_accepts_empty_required_scopes() {
    let identity = binding(&[], &[], Some("sp"));
    let subject = subject_of(classify_agent_subject(
        &identity,
        &delegated_claims("anything"),
        &AgentExecutionMode::Delegated,
    ));
    assert_eq!(subject.kind, SubjectKind::DelegatedSubject);
}

#[rstest]
#[case::delegated_autonomous_mode(
    AgentExecutionMode::Autonomous,
    "user",
    None,
    Some("user_impersonation"),
    DELEGATED_CONTRADICTION
)]
#[case::delegated_principal_oid(
    AgentExecutionMode::Delegated,
    "sp",
    None,
    Some("user_impersonation"),
    DELEGATED_CONTRADICTION
)]
#[case::delegated_idtyp_app(
    AgentExecutionMode::Delegated,
    "user",
    Some("app"),
    Some("user_impersonation"),
    DELEGATED_CONTRADICTION
)]
#[case::application_delegated_mode(
    AgentExecutionMode::Delegated,
    "sp",
    None,
    None,
    APPLICATION_CONTRADICTION
)]
#[case::application_other_oid(
    AgentExecutionMode::Autonomous,
    "other",
    None,
    None,
    APPLICATION_CONTRADICTION
)]
#[case::application_idtyp_user(
    AgentExecutionMode::Autonomous,
    "sp",
    Some("user"),
    None,
    APPLICATION_CONTRADICTION
)]
fn managed_classify_rejects_contradictions(
    #[case] mode: AgentExecutionMode,
    #[case] oid: &str,
    #[case] idtyp: Option<&str>,
    #[case] scope: Option<&str>,
    #[case] expected: &'static str,
) {
    let identity = binding(&["user_impersonation"], &[], Some("sp"));
    let token = claims_with(oid, idtyp, scope);
    let failure = failure_of(classify_agent_subject(&identity, &token, &mode));
    assert_eq!(failure.message, expected);
}

#[rstest]
#[case::missing_scope("other")]
#[case::whitespace_scope("   ")]
fn managed_classify_delegated_requires_scopes(#[case] scope: &str) {
    let identity = binding(&["user_impersonation"], &[], Some("sp"));
    let failure = failure_of(classify_agent_subject(
        &identity,
        &delegated_claims(scope),
        &AgentExecutionMode::Delegated,
    ));
    assert_eq!(failure.message, MISSING_SCOPES);
}

#[test]
fn managed_classify_rejects_non_text_scope() {
    let identity = binding(&["user_impersonation"], &[], Some("sp"));
    let token = claims(&[
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("user")),
        ("scp", number(5)),
    ]);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, INVALID_SCOPE);
}

#[test]
fn managed_classify_application_accepts_no_required_roles() {
    let identity = binding(&[], &[], Some("sp"));
    let subject = subject_of(classify_agent_subject(
        &identity,
        &application_claims(),
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(subject.kind, SubjectKind::Application);
}

#[test]
fn managed_classify_empty_scope_is_application() {
    let identity = binding(&[], &[], Some("sp"));
    let token = claims(&[
        ("iss", text(ISSUER)),
        ("tid", text("tenant")),
        ("azp", text("client")),
        ("oid", text("sp")),
        ("scp", text("")),
    ]);
    let subject = subject_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(subject.kind, SubjectKind::Application);
}

#[rstest]
#[case::missing_role(&["admin"], vec!["other"])]
#[case::empty_roles(&["admin"], vec![])]
fn managed_classify_application_requires_roles(
    #[case] required: &[&str],
    #[case] granted: Vec<&str>,
) {
    let identity = binding(&[], required, Some("sp"));
    let mut token = application_claims();
    token.0.insert(
        String::from("roles"),
        list(granted.iter().map(|role| text(role)).collect()),
    );
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, MISSING_ROLES);
}

#[rstest]
#[case::null_roles(JsonData::Null)]
#[case::text_roles(text("admin"))]
#[case::list_with_number(list(vec![number(1)]))]
fn managed_classify_rejects_invalid_roles(#[case] roles: JsonData) {
    let identity = binding(&[], &[], Some("sp"));
    let mut token = application_claims();
    token.0.insert(String::from("roles"), roles);
    let failure = failure_of(classify_agent_subject(
        &identity,
        &token,
        &AgentExecutionMode::Autonomous,
    ));
    assert_eq!(failure.message, INVALID_ROLES);
}
