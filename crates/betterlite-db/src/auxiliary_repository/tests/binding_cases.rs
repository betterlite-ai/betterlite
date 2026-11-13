use std::marker::PhantomData;

use rstest::rstest;

use super::super::bindings::{
    AccessGroupTable, AdaptiveRouterSession, AdaptiveRouterState, AgentIdentity, AgentsTable,
    AuditLog, BudgetWindowSpend, CacheConfig, ClaudeCodePluginTable, ConfigOverrides,
    DailyGuardrailMetrics, DailyGuardrailUsageUnits, DailyModelUsage, DailyPolicyMetrics,
    DailyTagSpend, DailyToolSpend, DeletedTeamTable, DeletedVerificationToken,
    DeprecatedVerificationToken, EndUserTable, GuardrailsTable, HealthCheckTable, InvitationLink,
    JWTKeyMapping, MCPServerOAuthClient, MCPServerTable, MCPToolsetTable, MCPUserCredentials,
    ManagedFileTable, ManagedObjectTable, ManagedVectorStoreIndexTable, ManagedVectorStoresTable,
    MemoryTable, ModelAccessGroupBudgetTable, ModelTable, ObjectPermissionTable,
    OrganizationMembership, PolicyAttachmentTable, PolicyTable, PromptTable, RetiredAgent,
    RetiredAgentIdentity, SSOConfig, SearchToolsTable, SkillsTable, SpendLogGuardrailIndex,
    SpendLogToolIndex, SpendLogs, TagTable, TeamMembership, TeamTable, ToolTable, UISettings,
    UserNotifications, VerifiedSubject, WorkflowEvent, WorkflowMessage, WorkflowRun,
};
use super::super::repository::TableBinding;

#[rstest]
#[case::policy(PhantomData::<PolicyTable>, "litellm_policytable", "LiteLLM_PolicyTable")]
#[case::agents(PhantomData::<AgentsTable>, "litellm_agentstable", "LiteLLM_AgentsTable")]
#[case::agent_identity(PhantomData::<AgentIdentity>, "litellm_agentidentity", "LiteLLM_AgentIdentity")]
#[case::retired_agent_identity(PhantomData::<RetiredAgentIdentity>, "litellm_retiredagentidentity", "LiteLLM_RetiredAgentIdentity")]
#[case::verified_subject(PhantomData::<VerifiedSubject>, "litellm_verifiedsubject", "LiteLLM_VerifiedSubject")]
#[case::object_permission(PhantomData::<ObjectPermissionTable>, "litellm_objectpermissiontable", "LiteLLM_ObjectPermissionTable")]
#[case::guardrails(PhantomData::<GuardrailsTable>, "litellm_guardrailstable", "LiteLLM_GuardrailsTable")]
#[case::mcp_server(PhantomData::<MCPServerTable>, "litellm_mcpservertable", "LiteLLM_MCPServerTable")]
#[case::managed_object(PhantomData::<ManagedObjectTable>, "litellm_managedobjecttable", "LiteLLM_ManagedObjectTable")]
#[case::organization_membership(PhantomData::<OrganizationMembership>, "litellm_organizationmembership", "LiteLLM_OrganizationMembership")]
#[case::spend_logs(PhantomData::<SpendLogs>, "litellm_spendlogs", "LiteLLM_SpendLogs")]
#[case::budget_window_spend(PhantomData::<BudgetWindowSpend>, "litellm_budgetwindowspend", "LiteLLM_BudgetWindowSpend")]
#[case::claude_code_plugin(PhantomData::<ClaudeCodePluginTable>, "litellm_claudecodeplugintable", "LiteLLM_ClaudeCodePluginTable")]
#[case::team_membership(PhantomData::<TeamMembership>, "litellm_teammembership", "LiteLLM_TeamMembership")]
#[case::end_user(PhantomData::<EndUserTable>, "litellm_endusertable", "LiteLLM_EndUserTable")]
#[case::managed_vector_stores(PhantomData::<ManagedVectorStoresTable>, "litellm_managedvectorstorestable", "LiteLLM_ManagedVectorStoresTable")]
#[case::mcp_user_credentials(PhantomData::<MCPUserCredentials>, "litellm_mcpusercredentials", "LiteLLM_MCPUserCredentials")]
#[case::mcp_server_oauth_client(PhantomData::<MCPServerOAuthClient>, "litellm_mcpserveroauthclient", "LiteLLM_MCPServerOAuthClient")]
#[case::prompt(PhantomData::<PromptTable>, "litellm_prompttable", "LiteLLM_PromptTable")]
#[case::tag(PhantomData::<TagTable>, "litellm_tagtable", "LiteLLM_TagTable")]
#[case::model_access_group_budget(PhantomData::<ModelAccessGroupBudgetTable>, "litellm_modelaccessgroupbudgettable", "LiteLLM_ModelAccessGroupBudgetTable")]
#[case::invitation_link(PhantomData::<InvitationLink>, "litellm_invitationlink", "LiteLLM_InvitationLink")]
#[case::jwt_key_mapping(PhantomData::<JWTKeyMapping>, "litellm_jwtkeymapping", "LiteLLM_JWTKeyMapping")]
#[case::managed_file(PhantomData::<ManagedFileTable>, "litellm_managedfiletable", "LiteLLM_ManagedFileTable")]
#[case::memory(PhantomData::<MemoryTable>, "litellm_memorytable", "LiteLLM_MemoryTable")]
#[case::search_tools(PhantomData::<SearchToolsTable>, "litellm_searchtoolstable", "LiteLLM_SearchToolsTable")]
#[case::config_overrides(PhantomData::<ConfigOverrides>, "litellm_configoverrides", "LiteLLM_ConfigOverrides")]
#[case::mcp_toolset(PhantomData::<MCPToolsetTable>, "litellm_mcptoolsettable", "LiteLLM_MCPToolsetTable")]
#[case::tool(PhantomData::<ToolTable>, "litellm_tooltable", "LiteLLM_ToolTable")]
#[case::deleted_verification_token(PhantomData::<DeletedVerificationToken>, "litellm_deletedverificationtoken", "LiteLLM_DeletedVerificationToken")]
#[case::workflow_run(PhantomData::<WorkflowRun>, "litellm_workflowrun", "LiteLLM_WorkflowRun")]
#[case::model(PhantomData::<ModelTable>, "litellm_modeltable", "LiteLLM_ModelTable")]
#[case::access_group(PhantomData::<AccessGroupTable>, "litellm_accessgrouptable", "LiteLLM_AccessGroupTable")]
#[case::sso_config(PhantomData::<SSOConfig>, "litellm_ssoconfig", "LiteLLM_SSOConfig")]
#[case::ui_settings(PhantomData::<UISettings>, "litellm_uisettings", "LiteLLM_UISettings")]
#[case::daily_guardrail_metrics(PhantomData::<DailyGuardrailMetrics>, "litellm_dailyguardrailmetrics", "LiteLLM_DailyGuardrailMetrics")]
#[case::daily_guardrail_usage_units(PhantomData::<DailyGuardrailUsageUnits>, "litellm_dailyguardrailusageunits", "LiteLLM_DailyGuardrailUsageUnits")]
#[case::policy_attachment(PhantomData::<PolicyAttachmentTable>, "litellm_policyattachmenttable", "LiteLLM_PolicyAttachmentTable")]
#[case::team(PhantomData::<TeamTable>, "litellm_teamtable", "LiteLLM_TeamTable")]
#[case::deleted_team(PhantomData::<DeletedTeamTable>, "litellm_deletedteamtable", "LiteLLM_DeletedTeamTable")]
#[case::skills(PhantomData::<SkillsTable>, "litellm_skillstable", "LiteLLM_SkillsTable")]
#[case::cache_config(PhantomData::<CacheConfig>, "litellm_cacheconfig", "LiteLLM_CacheConfig")]
#[case::managed_vector_store_index(PhantomData::<ManagedVectorStoreIndexTable>, "litellm_managedvectorstoreindextable", "LiteLLM_ManagedVectorStoreIndexTable")]
#[case::workflow_message(PhantomData::<WorkflowMessage>, "litellm_workflowmessage", "LiteLLM_WorkflowMessage")]
#[case::daily_tag_spend(PhantomData::<DailyTagSpend>, "litellm_dailytagspend", "LiteLLM_DailyTagSpend")]
#[case::spend_log_tool_index(PhantomData::<SpendLogToolIndex>, "litellm_spendlogtoolindex", "LiteLLM_SpendLogToolIndex")]
#[case::daily_tool_spend(PhantomData::<DailyToolSpend>, "litellm_dailytoolspend", "LiteLLM_DailyToolSpend")]
#[case::daily_model_usage(PhantomData::<DailyModelUsage>, "litellm_dailymodelusage", "LiteLLM_DailyModelUsage")]
#[case::spend_log_guardrail_index(PhantomData::<SpendLogGuardrailIndex>, "litellm_spendlogguardrailindex", "LiteLLM_SpendLogGuardrailIndex")]
#[case::user_notifications(PhantomData::<UserNotifications>, "litellm_usernotifications", "LiteLLM_UserNotifications")]
#[case::health_check(PhantomData::<HealthCheckTable>, "litellm_healthchecktable", "LiteLLM_HealthCheckTable")]
#[case::deprecated_verification_token(PhantomData::<DeprecatedVerificationToken>, "litellm_deprecatedverificationtoken", "LiteLLM_DeprecatedVerificationToken")]
#[case::workflow_event(PhantomData::<WorkflowEvent>, "litellm_workflowevent", "LiteLLM_WorkflowEvent")]
#[case::daily_policy_metrics(PhantomData::<DailyPolicyMetrics>, "litellm_dailypolicymetrics", "LiteLLM_DailyPolicyMetrics")]
#[case::adaptive_router_state(PhantomData::<AdaptiveRouterState>, "litellm_adaptiverouterstate", "LiteLLM_AdaptiveRouterState")]
#[case::audit_log(PhantomData::<AuditLog>, "litellm_auditlog", "LiteLLM_AuditLog")]
#[case::adaptive_router_session(PhantomData::<AdaptiveRouterSession>, "litellm_adaptiveroutersession", "LiteLLM_AdaptiveRouterSession")]
#[case::retired_agent(PhantomData::<RetiredAgent>, "litellm_retiredagent", "LiteLLM_RetiredAgent")]
fn table_binding<T: TableBinding>(
    #[case] _binding: PhantomData<T>,
    #[case] name: &str,
    #[case] table: &str,
) {
    assert_eq!(T::NAME, name);
    assert_eq!(T::TABLE, table);
    assert!(super::bindings::ALL.contains(&(T::NAME, T::TABLE)));
}

#[test]
fn binding_catalog_holds_every_listed_table() {
    assert_eq!(super::bindings::ALL.len(), 58);
}
