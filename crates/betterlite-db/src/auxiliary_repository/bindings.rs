use super::repository::TableBinding;

macro_rules! table_bindings {
    ($($binding:ident => $name:literal, $table:literal;)*) => {
        $(
            pub struct $binding;

            impl TableBinding for $binding {
                const NAME: &'static str = $name;
                const TABLE: &'static str = $table;
            }
        )*

        pub const ALL: &[(&str, &str)] = &[$(($name, $table)),*];
    };
}

table_bindings! {
    PolicyTable => "litellm_policytable", "LiteLLM_PolicyTable";
    AgentsTable => "litellm_agentstable", "LiteLLM_AgentsTable";
    AgentIdentity => "litellm_agentidentity", "LiteLLM_AgentIdentity";
    RetiredAgentIdentity => "litellm_retiredagentidentity", "LiteLLM_RetiredAgentIdentity";
    VerifiedSubject => "litellm_verifiedsubject", "LiteLLM_VerifiedSubject";
    ObjectPermissionTable => "litellm_objectpermissiontable", "LiteLLM_ObjectPermissionTable";
    GuardrailsTable => "litellm_guardrailstable", "LiteLLM_GuardrailsTable";
    MCPServerTable => "litellm_mcpservertable", "LiteLLM_MCPServerTable";
    ManagedObjectTable => "litellm_managedobjecttable", "LiteLLM_ManagedObjectTable";
    OrganizationMembership => "litellm_organizationmembership", "LiteLLM_OrganizationMembership";
    SpendLogs => "litellm_spendlogs", "LiteLLM_SpendLogs";
    BudgetWindowSpend => "litellm_budgetwindowspend", "LiteLLM_BudgetWindowSpend";
    ClaudeCodePluginTable => "litellm_claudecodeplugintable", "LiteLLM_ClaudeCodePluginTable";
    TeamMembership => "litellm_teammembership", "LiteLLM_TeamMembership";
    EndUserTable => "litellm_endusertable", "LiteLLM_EndUserTable";
    ManagedVectorStoresTable => "litellm_managedvectorstorestable", "LiteLLM_ManagedVectorStoresTable";
    MCPUserCredentials => "litellm_mcpusercredentials", "LiteLLM_MCPUserCredentials";
    MCPServerOAuthClient => "litellm_mcpserveroauthclient", "LiteLLM_MCPServerOAuthClient";
    PromptTable => "litellm_prompttable", "LiteLLM_PromptTable";
    TagTable => "litellm_tagtable", "LiteLLM_TagTable";
    ModelAccessGroupBudgetTable => "litellm_modelaccessgroupbudgettable", "LiteLLM_ModelAccessGroupBudgetTable";
    InvitationLink => "litellm_invitationlink", "LiteLLM_InvitationLink";
    JWTKeyMapping => "litellm_jwtkeymapping", "LiteLLM_JWTKeyMapping";
    ManagedFileTable => "litellm_managedfiletable", "LiteLLM_ManagedFileTable";
    MemoryTable => "litellm_memorytable", "LiteLLM_MemoryTable";
    SearchToolsTable => "litellm_searchtoolstable", "LiteLLM_SearchToolsTable";
    ConfigOverrides => "litellm_configoverrides", "LiteLLM_ConfigOverrides";
    MCPToolsetTable => "litellm_mcptoolsettable", "LiteLLM_MCPToolsetTable";
    ToolTable => "litellm_tooltable", "LiteLLM_ToolTable";
    DeletedVerificationToken => "litellm_deletedverificationtoken", "LiteLLM_DeletedVerificationToken";
    WorkflowRun => "litellm_workflowrun", "LiteLLM_WorkflowRun";
    ModelTable => "litellm_modeltable", "LiteLLM_ModelTable";
    AccessGroupTable => "litellm_accessgrouptable", "LiteLLM_AccessGroupTable";
    SSOConfig => "litellm_ssoconfig", "LiteLLM_SSOConfig";
    UISettings => "litellm_uisettings", "LiteLLM_UISettings";
    DailyGuardrailMetrics => "litellm_dailyguardrailmetrics", "LiteLLM_DailyGuardrailMetrics";
    DailyGuardrailUsageUnits => "litellm_dailyguardrailusageunits", "LiteLLM_DailyGuardrailUsageUnits";
    PolicyAttachmentTable => "litellm_policyattachmenttable", "LiteLLM_PolicyAttachmentTable";
    TeamTable => "litellm_teamtable", "LiteLLM_TeamTable";
    DeletedTeamTable => "litellm_deletedteamtable", "LiteLLM_DeletedTeamTable";
    SkillsTable => "litellm_skillstable", "LiteLLM_SkillsTable";
    CacheConfig => "litellm_cacheconfig", "LiteLLM_CacheConfig";
    ManagedVectorStoreIndexTable => "litellm_managedvectorstoreindextable", "LiteLLM_ManagedVectorStoreIndexTable";
    WorkflowMessage => "litellm_workflowmessage", "LiteLLM_WorkflowMessage";
    DailyTagSpend => "litellm_dailytagspend", "LiteLLM_DailyTagSpend";
    SpendLogToolIndex => "litellm_spendlogtoolindex", "LiteLLM_SpendLogToolIndex";
    DailyToolSpend => "litellm_dailytoolspend", "LiteLLM_DailyToolSpend";
    DailyModelUsage => "litellm_dailymodelusage", "LiteLLM_DailyModelUsage";
    SpendLogGuardrailIndex => "litellm_spendlogguardrailindex", "LiteLLM_SpendLogGuardrailIndex";
    UserNotifications => "litellm_usernotifications", "LiteLLM_UserNotifications";
    HealthCheckTable => "litellm_healthchecktable", "LiteLLM_HealthCheckTable";
    DeprecatedVerificationToken => "litellm_deprecatedverificationtoken", "LiteLLM_DeprecatedVerificationToken";
    WorkflowEvent => "litellm_workflowevent", "LiteLLM_WorkflowEvent";
    DailyPolicyMetrics => "litellm_dailypolicymetrics", "LiteLLM_DailyPolicyMetrics";
    AdaptiveRouterState => "litellm_adaptiverouterstate", "LiteLLM_AdaptiveRouterState";
    AuditLog => "litellm_auditlog", "LiteLLM_AuditLog";
    AdaptiveRouterSession => "litellm_adaptiveroutersession", "LiteLLM_AdaptiveRouterSession";
    RetiredAgent => "litellm_retiredagent", "LiteLLM_RetiredAgent";
}
