#![cfg(test)]

use rstest::rstest;

use super::common::CommonProxyErrors;
use super::{ModelAccessObject, ProxyErrorTypes, VectorStoreObject};

#[rstest]
#[case::budget_exceeded(ProxyErrorTypes::BudgetExceeded, "budget_exceeded")]
#[case::no_db_connection(ProxyErrorTypes::NoDbConnection, "no_db_connection")]
#[case::token_not_found_in_db(ProxyErrorTypes::TokenNotFoundInDb, "token_not_found_in_db")]
#[case::key_model_access_denied(ProxyErrorTypes::KeyModelAccessDenied, "key_model_access_denied")]
#[case::team_model_access_denied(
    ProxyErrorTypes::TeamModelAccessDenied,
    "team_model_access_denied"
)]
#[case::user_model_access_denied(
    ProxyErrorTypes::UserModelAccessDenied,
    "user_model_access_denied"
)]
#[case::org_model_access_denied(ProxyErrorTypes::OrgModelAccessDenied, "org_model_access_denied")]
#[case::project_model_access_denied(
    ProxyErrorTypes::ProjectModelAccessDenied,
    "project_model_access_denied"
)]
#[case::agent_model_access_denied(
    ProxyErrorTypes::AgentModelAccessDenied,
    "agent_model_access_denied"
)]
#[case::model_cost_map_missing(ProxyErrorTypes::ModelCostMapMissing, "model_cost_map_missing")]
#[case::expired_key(ProxyErrorTypes::ExpiredKey, "expired_key")]
#[case::auth_error(ProxyErrorTypes::AuthError, "auth_error")]
#[case::auth_provider_unavailable(
    ProxyErrorTypes::AuthProviderUnavailable,
    "auth_provider_unavailable"
)]
#[case::internal_server_error(ProxyErrorTypes::InternalServerError, "internal_server_error")]
#[case::bad_request_error(ProxyErrorTypes::BadRequestError, "bad_request_error")]
#[case::not_found_error(ProxyErrorTypes::NotFoundError, "not_found_error")]
#[case::validation_error(ProxyErrorTypes::ValidationError, "validation_error")]
#[case::cache_ping_error(ProxyErrorTypes::CachePingError, "cache_ping_error")]
#[case::team_member_permission_error(
    ProxyErrorTypes::TeamMemberPermissionError,
    "team_member_permission_error"
)]
#[case::key_vector_store_access_denied(
    ProxyErrorTypes::KeyVectorStoreAccessDenied,
    "key_vector_store_access_denied"
)]
#[case::team_vector_store_access_denied(
    ProxyErrorTypes::TeamVectorStoreAccessDenied,
    "team_vector_store_access_denied"
)]
#[case::org_vector_store_access_denied(
    ProxyErrorTypes::OrgVectorStoreAccessDenied,
    "org_vector_store_access_denied"
)]
#[case::team_member_already_in_team(
    ProxyErrorTypes::TeamMemberAlreadyInTeam,
    "team_member_already_in_team"
)]
#[case::tool_access_denied(ProxyErrorTypes::ToolAccessDenied, "tool_access_denied")]
fn every_error_type_renders_its_value(#[case] value: ProxyErrorTypes, #[case] text: &str) {
    assert_eq!(value.to_string(), text);
    assert_eq!(
        serde_json::to_value(value).unwrap(),
        serde_json::json!(text)
    );
    assert_eq!(
        serde_json::from_value::<ProxyErrorTypes>(serde_json::json!(text)).unwrap(),
        value
    );
}

#[rstest]
#[case::key(ModelAccessObject::Key, ProxyErrorTypes::KeyModelAccessDenied)]
#[case::team(ModelAccessObject::Team, ProxyErrorTypes::TeamModelAccessDenied)]
#[case::user(ModelAccessObject::User, ProxyErrorTypes::UserModelAccessDenied)]
#[case::org(ModelAccessObject::Org, ProxyErrorTypes::OrgModelAccessDenied)]
#[case::project(ModelAccessObject::Project, ProxyErrorTypes::ProjectModelAccessDenied)]
#[case::agent(ModelAccessObject::Agent, ProxyErrorTypes::AgentModelAccessDenied)]
fn model_access_denied_maps_every_object(
    #[case] object: ModelAccessObject,
    #[case] expected: ProxyErrorTypes,
) {
    assert_eq!(ProxyErrorTypes::model_access_denied_for(object), expected);
}

#[rstest]
#[case::key(VectorStoreObject::Key, ProxyErrorTypes::KeyVectorStoreAccessDenied)]
#[case::team(VectorStoreObject::Team, ProxyErrorTypes::TeamVectorStoreAccessDenied)]
#[case::org(VectorStoreObject::Org, ProxyErrorTypes::OrgVectorStoreAccessDenied)]
fn vector_store_access_denied_maps_every_object(
    #[case] object: VectorStoreObject,
    #[case] expected: ProxyErrorTypes,
) {
    assert_eq!(
        ProxyErrorTypes::vector_store_access_denied_for(object),
        expected
    );
}

#[rstest]
#[case::db_not_connected_error(
    CommonProxyErrors::DbNotConnectedError,
    "DB not connected. This endpoint needs a database; set DATABASE_URL to a PostgreSQL connection string (postgresql://...) to enable it. See https://docs.litellm.ai/docs/proxy/virtual_keys"
)]
#[case::no_llm_router(CommonProxyErrors::NoLlmRouter, "No models configured on proxy")]
#[case::not_allowed_access(
    CommonProxyErrors::NotAllowedAccess,
    "Admin-only endpoint. Not allowed to access this."
)]
#[case::not_premium_user(
    CommonProxyErrors::NotPremiumUser,
    "You must be a LiteLLM Enterprise user to use this feature. If you have a license please set `LITELLM_LICENSE` in your env. Get a 7 day trial key here: https://www.litellm.ai/enterprise#trial. \nPricing: https://www.litellm.ai/#pricing"
)]
#[case::max_parallel_request_limit_reached(
    CommonProxyErrors::MaxParallelRequestLimitReached,
    "Crossed TPM / RPM / Max Parallel Request Limit"
)]
#[case::missing_enterprise_package(
    CommonProxyErrors::MissingEnterprisePackage,
    "Missing litellm-enterprise package. Please install it to use this feature. Run `pip install litellm-enterprise`"
)]
#[case::missing_enterprise_package_docker(
    CommonProxyErrors::MissingEnterprisePackageDocker,
    "This uses the enterprise folder - only available on the Docker image."
)]
fn common_proxy_errors_keep_litellm_messages(
    #[case] error: CommonProxyErrors,
    #[case] expected: &str,
) {
    assert_eq!(error.as_str(), expected);
}
