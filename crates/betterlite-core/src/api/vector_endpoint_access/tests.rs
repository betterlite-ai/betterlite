#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::route_match::canonical::{RequestRouteSource, ScopeFields};
use betterlite_auth::user_roles::UserRole;
use betterlite_context::resource_config_selection::vector_stores::{
    VectorStoreFilesConfigTag, VectorStoresConfigTag,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_foundry::vector_stores::environment;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreIndexEndpoints;
use betterlite_openai::vector_stores::{file_endpoints, store_endpoints};
use rstest::rstest;

use super::error::VectorEndpointAccessError;
use super::{
    EndpointDecision, VectorEndpointAccess, VectorEndpointTables, VectorStorePermission,
    check_vector_store_permission, does_endpoint_match, file_endpoint_decision,
    store_endpoint_decision,
};

const IDX_READ: &str =
    r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["read"]}]}"#;
const IDX_WRITE: &str =
    r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["write"]}]}"#;
const IDX_BOTH: &str = r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["read", "write"]}]}"#;
const VS_READ: &str =
    r#"{"allowed_vector_store_indexes": [{"index_name": "vs_a", "index_permissions": ["read"]}]}"#;
const VS_WRITE: &str =
    r#"{"allowed_vector_store_indexes": [{"index_name": "vs_a", "index_permissions": ["write"]}]}"#;

struct ActualTables;

impl VectorEndpointTables for ActualTables {
    fn store_endpoints(&self, config: VectorStoresConfigTag) -> VectorStoreIndexEndpoints {
        match config {
            VectorStoresConfigTag::OpenAI => store_endpoints(),
            VectorStoresConfigTag::AzureAI => environment::endpoints_by_type(),
            VectorStoresConfigTag::AzureOpenAI
            | VectorStoresConfigTag::Vertex
            | VectorStoresConfigTag::VertexSearchAPI
            | VectorStoresConfigTag::Bedrock
            | VectorStoresConfigTag::PGVector
            | VectorStoresConfigTag::Milvus
            | VectorStoresConfigTag::Gemini
            | VectorStoresConfigTag::RAGFlow
            | VectorStoresConfigTag::S3Vectors
            | VectorStoresConfigTag::Valkey
            | VectorStoresConfigTag::MongoDB => empty_endpoints(),
        }
    }

    fn file_endpoints(&self, config: VectorStoreFilesConfigTag) -> VectorStoreIndexEndpoints {
        match config {
            VectorStoreFilesConfigTag::OpenAI => file_endpoints(),
        }
    }
}

static TABLES: ActualTables = ActualTables;

fn empty_endpoints() -> VectorStoreIndexEndpoints {
    VectorStoreIndexEndpoints {
        read: Some(Vec::new()),
        write: Some(Vec::new()),
        extra: BTreeMap::new(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Store,
    Files,
}

#[derive(Clone, Copy)]
struct Call {
    provider: ProviderId,
    id: &'static str,
    method: &'static str,
    path: &'static str,
    app_root_path: Option<&'static str>,
    root_path: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct Caller {
    role: Option<UserRole>,
    key_metadata: Option<&'static str>,
    team_metadata: Option<&'static str>,
}

impl Caller {
    fn auth(&self) -> UserAPIKeyAuth {
        let mut fields = BTreeMap::new();
        if let Some(role) = self.role {
            fields.insert(
                String::from("user_role"),
                JsonData::String(role.to_string()),
            );
        }
        if let Some(metadata) = self.key_metadata {
            fields.insert(
                String::from("metadata"),
                JsonData::Object(payload(metadata).0),
            );
        }
        if let Some(metadata) = self.team_metadata {
            fields.insert(
                String::from("team_metadata"),
                JsonData::Object(payload(metadata).0),
            );
        }
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Expect {
    Allowed,
    NotApplicable,
    StoreDenied(&'static str),
    FileDenied(&'static str),
    Admin(&'static str),
}

fn plain_call(
    provider: ProviderId,
    id: &'static str,
    method: &'static str,
    path: &'static str,
) -> Call {
    Call {
        provider,
        id,
        method,
        path,
        app_root_path: None,
        root_path: None,
    }
}

fn anonymous(key_metadata: Option<&'static str>, team_metadata: Option<&'static str>) -> Caller {
    Caller {
        role: None,
        key_metadata,
        team_metadata,
    }
}

fn admin() -> Caller {
    Caller {
        role: Some(UserRole::ProxyAdmin),
        key_metadata: None,
        team_metadata: None,
    }
}

fn payload(text: &str) -> Payload {
    serde_json::from_str(text).unwrap()
}

fn access_for<'access>(
    call: &'access Call,
    auth: &'access UserAPIKeyAuth,
    tables: &'access dyn VectorEndpointTables,
) -> VectorEndpointAccess<'access> {
    VectorEndpointAccess {
        provider: call.provider,
        vector_store_id: call.id,
        method: call.method,
        route: RequestRouteSource {
            scope: Some(ScopeFields {
                path: Some(call.path),
                app_root_path: call.app_root_path,
                root_path: call.root_path,
            }),
            url_path: call.path,
        },
        auth,
        tables,
    }
}

fn decide(
    kind: Kind,
    access: &VectorEndpointAccess<'_>,
) -> Result<EndpointDecision, VectorEndpointAccessError> {
    match kind {
        Kind::Store => store_endpoint_decision(access),
        Kind::Files => file_endpoint_decision(access),
    }
}

fn assert_expect(result: Result<EndpointDecision, VectorEndpointAccessError>, expect: Expect) {
    match expect {
        Expect::Allowed => assert_eq!(result.unwrap(), EndpointDecision::Allowed),
        Expect::NotApplicable => assert_eq!(result.unwrap(), EndpointDecision::NotApplicable),
        Expect::StoreDenied(index_name) => {
            let error = result.unwrap_err();
            assert_eq!(error.status(), 403);
            assert_eq!(error.to_string(), store_denied(index_name));
        }
        Expect::FileDenied(vector_store_id) => {
            let error = result.unwrap_err();
            assert_eq!(error.status(), 403);
            assert_eq!(error.to_string(), file_denied(vector_store_id));
        }
        Expect::Admin(operation) => {
            let error = result.unwrap_err();
            assert_eq!(error.status(), 403);
            assert_eq!(error.to_string(), admin_denied(operation));
        }
    }
}

fn store_denied(index_name: &str) -> String {
    format!(
        "User does not have permission to call vector store endpoint {index_name}. Ask your administrator to add the necessary permissions to your API key/Team."
    )
}

fn file_denied(vector_store_id: &str) -> String {
    format!(
        "User does not have permission to call vector store file endpoint {vector_store_id}. Ask your administrator to add the necessary permissions to your API key/Team."
    )
}

fn admin_denied(operation: &str) -> String {
    format!(
        "Only proxy admins can {operation} vector store indexes. Contact your LiteLLM administrator."
    )
}

fn run_case(kind: Kind, call: &Call, caller: &Caller, expect: Expect) {
    let auth = caller.auth();
    let call_access = access_for(call, &auth, &TABLES);
    assert_expect(decide(kind, &call_access), expect);
}

#[rstest]
#[case::literal_inside_a_segment(
    Kind::Store,
    plain_call(ProviderId::AzureAi, "idx", "POST", "/indexes/idx/docs/searchExtra"),
    anonymous(Some(IDX_READ), None),
    Expect::Allowed
)]
#[case::placeholder_prefix_matches_any_suffix(
    Kind::Files,
    plain_call(ProviderId::Openai, "vs_a", "GET", "/vector_stores/vs_a/whatever"),
    anonymous(Some(VS_READ), None),
    Expect::Allowed
)]
#[case::method_is_matched_exactly(
    Kind::Store,
    plain_call(ProviderId::Openai, "idx", "get", "/vector_stores/idx/search"),
    anonymous(Some(IDX_BOTH), None),
    Expect::StoreDenied("idx")
)]
#[case::literal_is_matched_exactly(
    Kind::Files,
    plain_call(ProviderId::Openai, "vs_a", "GET", "/vector_stores/vs_a/filesXYZ"),
    anonymous(Some(VS_READ), None),
    Expect::Allowed
)]
#[case::unmatched_store_route_denies(
    Kind::Store,
    plain_call(ProviderId::Openai, "idx", "GET", "/embeddings/idx"),
    anonymous(Some(IDX_BOTH), None),
    Expect::StoreDenied("idx")
)]
fn endpoint_pattern_matching(
    #[case] kind: Kind,
    #[case] call: Call,
    #[case] caller: Caller,
    #[case] expect: Expect,
) {
    run_case(kind, &call, &caller, expect);
}

#[rstest]
#[case::app_root_removed_before_matching(
    Kind::Files,
    Call {
        provider: ProviderId::Openai,
        id: "vs_a",
        method: "GET",
        path: "/vector_stores/vs_a/files/file_a",
        app_root_path: Some("/vector_stores"),
        root_path: None,
    },
    Expect::NotApplicable
)]
#[case::root_path_removed_before_matching(
    Kind::Files,
    Call {
        provider: ProviderId::Openai,
        id: "vs_a",
        method: "GET",
        path: "/vector_stores/vs_a/files/file_a",
        app_root_path: None,
        root_path: Some("/vector_stores"),
    },
    Expect::NotApplicable
)]
#[case::app_root_trailing_slash_is_trimmed(
    Kind::Files,
    Call {
        provider: ProviderId::Openai,
        id: "vs_a",
        method: "GET",
        path: "/vector_stores/vs_a/files",
        app_root_path: Some("/vector_stores/"),
        root_path: None,
    },
    Expect::NotApplicable
)]
#[case::app_root_wins_over_root_path(
    Kind::Files,
    Call {
        provider: ProviderId::Openai,
        id: "vs_a",
        method: "GET",
        path: "/vector_stores/vs_a/files",
        app_root_path: Some("/api"),
        root_path: Some("/vector_stores"),
    },
    Expect::Allowed
)]
fn endpoint_pattern_matching_uses_the_canonical_route(
    #[case] kind: Kind,
    #[case] call: Call,
    #[case] expect: Expect,
) {
    run_case(kind, &call, &anonymous(Some(VS_READ), None), expect);
}

#[rstest]
#[case::literal_anywhere("/search", "/v1/searching", true)]
#[case::literal_at_a_boundary("/vector_stores", "/vector_stores/idx", true)]
#[case::placeholder_prefix("/vector_stores/{id}/files", "/vector_stores/x/other", true)]
#[case::brace_leading_pattern_needs_the_literal("{id}/files", "/v1/{id}/files", true)]
#[case::brace_leading_pattern_has_no_prefix("{id}/files", "/v1/abc/files", false)]
#[case::empty_prefix_matches_nothing("{id}", "/anything", false)]
#[case::empty_pattern_matches_every_route("", "/anything", true)]
#[case::literal_missing("/search", "/retrieve", false)]
fn does_endpoint_match_cases(#[case] pattern: &str, #[case] route: &str, #[case] expected: bool) {
    assert_eq!(does_endpoint_match(pattern, route), expected);
}

#[rstest]
#[case::key_read_grant(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["read"]}]}"#),
    None,
    VectorStorePermission::Read,
    true
)]
#[case::write_grant_does_not_imply_read(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["write"]}]}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::key_write_grant(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["write"]}]}"#),
    None,
    VectorStorePermission::Write,
    true
)]
#[case::team_write_grant(
    None,
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["write"]}]}"#),
    VectorStorePermission::Write,
    true
)]
#[case::team_read_grant_does_not_imply_write(
    None,
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["read"]}]}"#),
    VectorStorePermission::Write,
    false
)]
#[case::missing_metadata(None, None, VectorStorePermission::Read, false)]
#[case::null_metadata(None, None, VectorStorePermission::Write, false)]
#[case::empty_metadata_object(Some("{}"), None, VectorStorePermission::Read, false)]
#[case::allowed_indexes_is_not_a_list(
    Some(r#"{"allowed_vector_store_indexes": "idx"}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::allowed_indexes_is_an_object(
    Some(
        r#"{"allowed_vector_store_indexes": {"index_name": "idx", "index_permissions": ["read"]}}"#
    ),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::empty_allowed_indexes(
    Some(r#"{"allowed_vector_store_indexes": []}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::non_dictionary_entries_are_skipped(
    Some(r#"{"allowed_vector_store_indexes": [7, "idx", {"index_name": "idx", "index_permissions": ["read"]}]}"#),
    None,
    VectorStorePermission::Read,
    true
)]
#[case::only_non_dictionary_entries(
    Some(r#"{"allowed_vector_store_indexes": [7, "idx", null]}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::wrong_index_name(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx2", "index_permissions": ["read"]}]}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::wildcard_index_name(
    Some(
        r#"{"allowed_vector_store_indexes": [{"index_name": "*", "index_permissions": ["read"]}]}"#
    ),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::permissions_are_not_a_list(
    Some(
        r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": "read"}]}"#
    ),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::permission_strings_are_exact(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["READ"]}]}"#),
    None,
    VectorStorePermission::Read,
    false
)]
#[case::exact_permission_in_a_later_entry(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx", "index_permissions": ["READ", "read"]}]}"#),
    None,
    VectorStorePermission::Read,
    true
)]
#[case::missing_permissions_key(
    Some(r#"{"allowed_vector_store_indexes": [{"index_name": "idx"}]}"#),
    None,
    VectorStorePermission::Read,
    false
)]
fn index_grant_metadata(
    #[case] key_metadata: Option<&str>,
    #[case] team_metadata: Option<&str>,
    #[case] permission: VectorStorePermission,
    #[case] expected: bool,
) {
    let key = key_metadata.map(payload);
    let team = team_metadata.map(payload);
    assert_eq!(
        check_vector_store_permission("idx", permission, key.as_ref(), team.as_ref()),
        expected
    );
}

#[test]
fn index_grant_metadata_reaches_the_store_endpoint() {
    let call = plain_call(ProviderId::Openai, "idx", "POST", "/vector_stores");
    let caller = anonymous(Some(IDX_READ), Some(IDX_WRITE));
    run_case(Kind::Store, &call, &caller, Expect::Allowed);
}

#[rstest]
#[case::batch_write_needs_write(
    plain_call(ProviderId::AzureAi, "idx", "POST", "/indexes/analyze-x/docs/index"),
    anonymous(Some(IDX_READ), None),
    Expect::StoreDenied("idx")
)]
#[case::batch_write_with_write_grant(
    plain_call(ProviderId::AzureAi, "idx", "POST", "/indexes/analyze-x/docs/index"),
    anonymous(Some(IDX_WRITE), None),
    Expect::Allowed
)]
#[case::analyze_search_is_still_a_read(
    plain_call(ProviderId::AzureAi, "idx", "POST", "/indexes/analyze-x/docs/search"),
    anonymous(Some(IDX_READ), None),
    Expect::Allowed
)]
#[case::lifecycle_delete_denied(
    plain_call(ProviderId::AzureAi, "idx", "DELETE", "/indexes/idx"),
    anonymous(Some(IDX_BOTH), None),
    Expect::Admin("delete")
)]
#[case::lifecycle_put_denied(
    plain_call(ProviderId::AzureAi, "idx", "PUT", "/indexes/idx"),
    anonymous(Some(IDX_BOTH), None),
    Expect::Admin("update")
)]
#[case::lifecycle_patch_denied(
    plain_call(ProviderId::AzureAi, "idx", "PATCH", "/indexes/idx"),
    anonymous(Some(IDX_BOTH), None),
    Expect::Admin("update")
)]
#[case::lifecycle_create_denied(
    plain_call(ProviderId::AzureAi, "idx", "POST", "/indexes"),
    anonymous(Some(IDX_BOTH), None),
    Expect::Admin("create")
)]
#[case::lifecycle_admin_passes(
    plain_call(ProviderId::AzureAi, "idx", "DELETE", "/indexes/idx"),
    admin(),
    Expect::Allowed
)]
#[case::admin_before_provider_lookup(
    plain_call(ProviderId::Cohere, "idx", "DELETE", "/indexes/idx"),
    admin(),
    Expect::Allowed
)]
#[case::unsupported_provider_is_null(
    plain_call(ProviderId::Cohere, "idx", "POST", "/indexes/idx/docs/search"),
    anonymous(Some(IDX_BOTH), None),
    Expect::NotApplicable
)]
#[case::unmatched_store_endpoint_denies(
    plain_call(ProviderId::Openai, "idx", "GET", "/embeddings/idx"),
    anonymous(Some(IDX_BOTH), None),
    Expect::StoreDenied("idx")
)]
#[case::missing_write_grant_denies(
    plain_call(ProviderId::Openai, "idx", "POST", "/vector_stores"),
    anonymous(Some(IDX_READ), None),
    Expect::StoreDenied("idx")
)]
#[case::admin_ignores_grants(
    plain_call(ProviderId::Openai, "idx", "POST", "/vector_stores"),
    admin(),
    Expect::Allowed
)]
fn write_before_read_and_lifecycle(
    #[case] call: Call,
    #[case] caller: Caller,
    #[case] expect: Expect,
) {
    run_case(Kind::Store, &call, &caller, expect);
}

#[rstest]
#[case::read_retrieval(
    plain_call(ProviderId::Openai, "vs_a", "GET", "/vector_stores/vs_a/files/file_a"),
    anonymous(Some(VS_READ), None),
    Expect::Allowed
)]
#[case::read_content(
    plain_call(
        ProviderId::Openai,
        "vs_a",
        "GET",
        "/vector_stores/vs_a/files/file_a/content"
    ),
    anonymous(Some(VS_READ), None),
    Expect::Allowed
)]
#[case::update_needs_write(
    plain_call(ProviderId::Openai, "vs_a", "POST", "/vector_stores/vs_a/files/file_a"),
    anonymous(Some(VS_READ), None),
    Expect::FileDenied("vs_a")
)]
#[case::attach_with_write(
    plain_call(ProviderId::Openai, "vs_a", "POST", "/vector_stores/vs_a/files"),
    anonymous(Some(VS_WRITE), None),
    Expect::Allowed
)]
#[case::detach_with_write(
    plain_call(
        ProviderId::Openai,
        "vs_a",
        "DELETE",
        "/vector_stores/vs_a/files/file_a"
    ),
    anonymous(Some(VS_WRITE), None),
    Expect::Allowed
)]
#[case::missing_grant_denies(
    plain_call(ProviderId::Openai, "vs_a", "GET", "/vector_stores/vs_a/files/file_a"),
    anonymous(None, None),
    Expect::FileDenied("vs_a")
)]
#[case::unmatched_file_endpoint_is_null(
    plain_call(ProviderId::Openai, "vs_a", "GET", "/embeddings/vs_a"),
    anonymous(Some(VS_READ), None),
    Expect::NotApplicable
)]
#[case::unsupported_file_provider_is_null(
    plain_call(ProviderId::AzureAi, "vs_a", "GET", "/embeddings/vs_a"),
    anonymous(Some(VS_READ), None),
    Expect::NotApplicable
)]
#[case::admin_passes_unmatched_endpoint(
    plain_call(ProviderId::Openai, "vs_a", "GET", "/embeddings/vs_a"),
    admin(),
    Expect::Allowed
)]
#[case::admin_before_file_provider_lookup(
    plain_call(ProviderId::AzureAi, "vs_a", "GET", "/embeddings/vs_a"),
    admin(),
    Expect::Allowed
)]
fn attachment_endpoint_policy(#[case] call: Call, #[case] caller: Caller, #[case] expect: Expect) {
    run_case(Kind::Files, &call, &caller, expect);
}

#[test]
fn store_unknown_endpoint_denies_where_files_returns_null() {
    let call = plain_call(ProviderId::Openai, "vs_a", "GET", "/embeddings/vs_a");
    run_case(
        Kind::Store,
        &call,
        &anonymous(Some(VS_READ), None),
        Expect::StoreDenied("vs_a"),
    );
    run_case(
        Kind::Files,
        &call,
        &anonymous(Some(VS_READ), None),
        Expect::NotApplicable,
    );
}
