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
#[case::table_binding_policytable(PhantomData::<PolicyTable>, "litellm_policytable", "LiteLLM_PolicyTable")]
#[case::table_binding_agentstable(PhantomData::<AgentsTable>, "litellm_agentstable", "LiteLLM_AgentsTable")]
#[case::table_binding_agentidentity(PhantomData::<AgentIdentity>, "litellm_agentidentity", "LiteLLM_AgentIdentity")]
#[case::table_binding_retiredagentidentity(PhantomData::<RetiredAgentIdentity>, "litellm_retiredagentidentity", "LiteLLM_RetiredAgentIdentity")]
#[case::table_binding_verifiedsubject(PhantomData::<VerifiedSubject>, "litellm_verifiedsubject", "LiteLLM_VerifiedSubject")]
#[case::table_binding_objectpermissiontable(PhantomData::<ObjectPermissionTable>, "litellm_objectpermissiontable", "LiteLLM_ObjectPermissionTable")]
#[case::table_binding_guardrailstable(PhantomData::<GuardrailsTable>, "litellm_guardrailstable", "LiteLLM_GuardrailsTable")]
#[case::table_binding_mcpservertable(PhantomData::<MCPServerTable>, "litellm_mcpservertable", "LiteLLM_MCPServerTable")]
#[case::table_binding_managedobjecttable(PhantomData::<ManagedObjectTable>, "litellm_managedobjecttable", "LiteLLM_ManagedObjectTable")]
#[case::table_binding_organizationmembership(PhantomData::<OrganizationMembership>, "litellm_organizationmembership", "LiteLLM_OrganizationMembership")]
#[case::table_binding_spendlogs(PhantomData::<SpendLogs>, "litellm_spendlogs", "LiteLLM_SpendLogs")]
#[case::table_binding_budgetwindowspend(PhantomData::<BudgetWindowSpend>, "litellm_budgetwindowspend", "LiteLLM_BudgetWindowSpend")]
#[case::table_binding_claudecodeplugintable(PhantomData::<ClaudeCodePluginTable>, "litellm_claudecodeplugintable", "LiteLLM_ClaudeCodePluginTable")]
#[case::table_binding_teammembership(PhantomData::<TeamMembership>, "litellm_teammembership", "LiteLLM_TeamMembership")]
#[case::table_binding_endusertable(PhantomData::<EndUserTable>, "litellm_endusertable", "LiteLLM_EndUserTable")]
#[case::table_binding_managedvectorstorestable(PhantomData::<ManagedVectorStoresTable>, "litellm_managedvectorstorestable", "LiteLLM_ManagedVectorStoresTable")]
#[case::table_binding_mcpusercredentials(PhantomData::<MCPUserCredentials>, "litellm_mcpusercredentials", "LiteLLM_MCPUserCredentials")]
#[case::table_binding_mcpserveroauthclient(PhantomData::<MCPServerOAuthClient>, "litellm_mcpserveroauthclient", "LiteLLM_MCPServerOAuthClient")]
#[case::table_binding_prompttable(PhantomData::<PromptTable>, "litellm_prompttable", "LiteLLM_PromptTable")]
#[case::table_binding_tagtable(PhantomData::<TagTable>, "litellm_tagtable", "LiteLLM_TagTable")]
#[case::table_binding_modelaccessgroupbudgettable(PhantomData::<ModelAccessGroupBudgetTable>, "litellm_modelaccessgroupbudgettable", "LiteLLM_ModelAccessGroupBudgetTable")]
#[case::table_binding_invitationlink(PhantomData::<InvitationLink>, "litellm_invitationlink", "LiteLLM_InvitationLink")]
#[case::table_binding_jwtkeymapping(PhantomData::<JWTKeyMapping>, "litellm_jwtkeymapping", "LiteLLM_JWTKeyMapping")]
#[case::table_binding_managedfiletable(PhantomData::<ManagedFileTable>, "litellm_managedfiletable", "LiteLLM_ManagedFileTable")]
#[case::table_binding_memorytable(PhantomData::<MemoryTable>, "litellm_memorytable", "LiteLLM_MemoryTable")]
#[case::table_binding_searchtoolstable(PhantomData::<SearchToolsTable>, "litellm_searchtoolstable", "LiteLLM_SearchToolsTable")]
#[case::table_binding_configoverrides(PhantomData::<ConfigOverrides>, "litellm_configoverrides", "LiteLLM_ConfigOverrides")]
#[case::table_binding_mcptoolsettable(PhantomData::<MCPToolsetTable>, "litellm_mcptoolsettable", "LiteLLM_MCPToolsetTable")]
#[case::table_binding_tooltable(PhantomData::<ToolTable>, "litellm_tooltable", "LiteLLM_ToolTable")]
#[case::table_binding_deletedverificationtoken(PhantomData::<DeletedVerificationToken>, "litellm_deletedverificationtoken", "LiteLLM_DeletedVerificationToken")]
#[case::table_binding_workflowrun(PhantomData::<WorkflowRun>, "litellm_workflowrun", "LiteLLM_WorkflowRun")]
#[case::table_binding_modeltable(PhantomData::<ModelTable>, "litellm_modeltable", "LiteLLM_ModelTable")]
#[case::table_binding_accessgrouptable(PhantomData::<AccessGroupTable>, "litellm_accessgrouptable", "LiteLLM_AccessGroupTable")]
#[case::table_binding_ssoconfig(PhantomData::<SSOConfig>, "litellm_ssoconfig", "LiteLLM_SSOConfig")]
#[case::table_binding_uisettings(PhantomData::<UISettings>, "litellm_uisettings", "LiteLLM_UISettings")]
#[case::table_binding_dailyguardrailmetrics(PhantomData::<DailyGuardrailMetrics>, "litellm_dailyguardrailmetrics", "LiteLLM_DailyGuardrailMetrics")]
#[case::table_binding_dailyguardrailusageunits(PhantomData::<DailyGuardrailUsageUnits>, "litellm_dailyguardrailusageunits", "LiteLLM_DailyGuardrailUsageUnits")]
#[case::table_binding_policyattachmenttable(PhantomData::<PolicyAttachmentTable>, "litellm_policyattachmenttable", "LiteLLM_PolicyAttachmentTable")]
#[case::table_binding_teamtable(PhantomData::<TeamTable>, "litellm_teamtable", "LiteLLM_TeamTable")]
#[case::table_binding_deletedteamtable(PhantomData::<DeletedTeamTable>, "litellm_deletedteamtable", "LiteLLM_DeletedTeamTable")]
#[case::table_binding_skillstable(PhantomData::<SkillsTable>, "litellm_skillstable", "LiteLLM_SkillsTable")]
#[case::table_binding_cacheconfig(PhantomData::<CacheConfig>, "litellm_cacheconfig", "LiteLLM_CacheConfig")]
#[case::table_binding_managedvectorstoreindextable(PhantomData::<ManagedVectorStoreIndexTable>, "litellm_managedvectorstoreindextable", "LiteLLM_ManagedVectorStoreIndexTable")]
#[case::table_binding_workflowmessage(PhantomData::<WorkflowMessage>, "litellm_workflowmessage", "LiteLLM_WorkflowMessage")]
#[case::table_binding_dailytagspend(PhantomData::<DailyTagSpend>, "litellm_dailytagspend", "LiteLLM_DailyTagSpend")]
#[case::table_binding_spendlogtoolindex(PhantomData::<SpendLogToolIndex>, "litellm_spendlogtoolindex", "LiteLLM_SpendLogToolIndex")]
#[case::table_binding_dailytoolspend(PhantomData::<DailyToolSpend>, "litellm_dailytoolspend", "LiteLLM_DailyToolSpend")]
#[case::table_binding_dailymodelusage(PhantomData::<DailyModelUsage>, "litellm_dailymodelusage", "LiteLLM_DailyModelUsage")]
#[case::table_binding_spendlogguardrailindex(PhantomData::<SpendLogGuardrailIndex>, "litellm_spendlogguardrailindex", "LiteLLM_SpendLogGuardrailIndex")]
#[case::table_binding_usernotifications(PhantomData::<UserNotifications>, "litellm_usernotifications", "LiteLLM_UserNotifications")]
#[case::table_binding_healthchecktable(PhantomData::<HealthCheckTable>, "litellm_healthchecktable", "LiteLLM_HealthCheckTable")]
#[case::table_binding_deprecatedverificationtoken(PhantomData::<DeprecatedVerificationToken>, "litellm_deprecatedverificationtoken", "LiteLLM_DeprecatedVerificationToken")]
#[case::table_binding_workflowevent(PhantomData::<WorkflowEvent>, "litellm_workflowevent", "LiteLLM_WorkflowEvent")]
#[case::table_binding_dailypolicymetrics(PhantomData::<DailyPolicyMetrics>, "litellm_dailypolicymetrics", "LiteLLM_DailyPolicyMetrics")]
#[case::table_binding_adaptiverouterstate(PhantomData::<AdaptiveRouterState>, "litellm_adaptiverouterstate", "LiteLLM_AdaptiveRouterState")]
#[case::table_binding_auditlog(PhantomData::<AuditLog>, "litellm_auditlog", "LiteLLM_AuditLog")]
#[case::table_binding_adaptiveroutersession(PhantomData::<AdaptiveRouterSession>, "litellm_adaptiveroutersession", "LiteLLM_AdaptiveRouterSession")]
#[case::table_binding_retiredagent(PhantomData::<RetiredAgent>, "litellm_retiredagent", "LiteLLM_RetiredAgent")]
fn table_binding<T>(#[case] binding: PhantomData<T>, #[case] name: &str, #[case] table: &str)
where
    T: TableBinding,
{
    assert_eq!(T::NAME, name, "binding {binding:?}");
    assert_eq!(T::TABLE, table, "binding {binding:?}");
    assert!(super::super::bindings::ALL.contains(&(T::NAME, T::TABLE)));
}

#[test]
fn binding_catalog_holds_every_listed_table() {
    assert_eq!(super::super::bindings::ALL.len(), 58);
}
