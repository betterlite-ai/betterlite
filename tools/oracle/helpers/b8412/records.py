from typing import TypeAlias, TypedDict

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True

Amount: TypeAlias = int | float


class BudgetDump(TypedDict):
    budget_id: str | None
    soft_budget: Amount | None
    max_budget: Amount | None
    max_parallel_requests: int | None
    tpm_limit: int | None
    rpm_limit: int | None
    tpd_limit: int | None
    model_max_budget: dict[str, JsonValue] | None
    budget_duration: str | None
    allowed_models: list[str] | None
    temp_budget_increase: Amount | None
    temp_budget_expiry: str | None


class PermissionDump(TypedDict, total=False):
    object_permission_id: str
    mcp_servers: list[str] | None
    mcp_access_groups: list[str] | None
    mcp_tool_permissions: dict[str, list[str]] | None
    mcp_toolsets: list[str] | None
    blocked_tools: list[str] | None
    vector_stores: list[str] | None
    agents: list[str] | None
    agent_access_groups: list[str] | None
    models: list[str] | None
    search_tools: list[str] | None
    mcp_tool_search_enabled: bool | None
    skills: list[str] | None


class MemberDump(TypedDict):
    user_id: str | None
    user_email: str | None
    role: str


class BudgetLimitDump(TypedDict):
    budget_duration: str
    max_budget: Amount
    reset_at: str | None


class MembershipDump(TypedDict):
    user_id: str
    organization_id: str
    user_role: str | None
    spend: Amount
    budget_id: str | None
    created_at: str
    updated_at: str
    user: JsonValue
    litellm_budget_table: BudgetDump | None
    user_email: str | None


class ModelTableDump(TypedDict):
    id: int | None
    model_aliases: str | dict[str, JsonValue] | None
    created_by: str
    updated_by: str
    team: "TeamDump | None"


class TeamDump(TypedDict):
    team_alias: str | None
    team_id: str
    organization_id: str | None
    admins: list[str]
    members: list[str]
    members_with_roles: list[MemberDump]
    team_member_permissions: list[str] | None
    metadata: dict[str, JsonValue] | None
    tpm_limit: int | None
    rpm_limit: int | None
    tpd_limit: int | None
    max_budget: Amount | None
    soft_budget: Amount | None
    budget_duration: str | None
    budget_limits: list[BudgetLimitDump] | None
    models: list[str]
    blocked: bool
    router_settings: dict[str, JsonValue] | None
    access_group_ids: list[str] | None
    default_team_member_models: list[str] | None
    spend: Amount | None
    max_parallel_requests: int | None
    budget_reset_at: str | None
    model_id: int | None
    model_spend: dict[str, JsonValue] | None
    model_max_budget: dict[str, JsonValue] | None
    policies: list[str] | None
    allow_team_guardrail_config: bool | None
    litellm_model_table: ModelTableDump | None
    object_permission: PermissionDump | None
    object_permission_id: str | None
    updated_at: str | None
    created_at: str | None


class UserDump(TypedDict):
    user_id: str
    user_alias: str | None
    team_id: str | None
    sso_user_id: str | None
    organization_id: str | None
    object_permission_id: str | None
    password_reset_required: bool | None
    last_breach_check_at: str | None
    teams: list[str]
    user_role: str | None
    max_budget: Amount | None
    spend: Amount
    user_email: str | None
    models: list[JsonValue]
    metadata: dict[str, JsonValue] | None
    max_parallel_requests: int | None
    tpm_limit: int | None
    rpm_limit: int | None
    budget_duration: str | None
    budget_reset_at: str | None
    allowed_cache_controls: list[str]
    policies: list[str]
    model_spend: dict[str, JsonValue] | None
    model_max_budget: dict[str, JsonValue] | None
    created_at: str | None
    updated_at: str | None
    organization_memberships: list[MembershipDump] | None
    object_permission: PermissionDump | None


class DeleteTeamDump(TypedDict):
    team_ids: list[str]


class AddTeamCallbackDump(TypedDict):
    callback_name: str
    callback_type: str | None
    callback_vars: dict[str, str]


class TeamCallbackMetadataDump(TypedDict):
    success_callback: list[str]
    failure_callback: list[str]
    callbacks: list[str]
    callback_vars: dict[str, str]


class MemberAddDump(TypedDict):
    member: MemberDump | list[MemberDump]


class TeamMemberAddDump(MemberAddDump):
    team_id: str
    max_budget_in_team: Amount | None
    budget_duration: str | None
    allowed_models: list[str] | None


class OrganizationMemberAddDump(MemberAddDump):
    organization_id: str
    max_budget_in_organization: Amount | None


class TeamMemberDeleteDump(TypedDict):
    user_id: str | None
    user_email: str | None
    team_id: str


class OrganizationMemberDeleteDump(TypedDict):
    user_id: str | None
    user_email: str | None
    organization_id: str


class TeamMemberUpdateDump(TeamMemberDeleteDump):
    max_budget_in_team: Amount | None
    role: str | None
    tpm_limit: int | None
    rpm_limit: int | None
    budget_duration: str | None
    allowed_models: list[str] | None
    temp_budget_increase: Amount | None
    temp_budget_expiry: str | None


class OrganizationMemberUpdateDump(OrganizationMemberDeleteDump):
    max_budget_in_organization: Amount | None
    role: str | None


class OrgTableUpdateDump(BudgetDump):
    organization_id: str | None
    organization_alias: str | None
    spend: Amount | None
    metadata: dict[str, JsonValue] | None
    models: list[str] | None
    updated_by: str | None
    object_permission: PermissionDump | None
    model_tpm_limit: dict[str, int] | None
    model_rpm_limit: dict[str, int] | None


class OrgUpdateV2Dump(TypedDict):
    organization_alias: str | None
    models: list[str] | None
    metadata: dict[str, JsonValue] | None
    tpm_limit: int | None
    rpm_limit: int | None
    max_budget: Amount | None
    soft_budget: Amount | None
    max_parallel_requests: int | None
    model_max_budget: dict[str, JsonValue] | None
    budget_duration: str | None
    object_permission: PermissionDump | None


class OrgTableWithMembersDump(TypedDict):
    organization_id: str | None
    organization_alias: str | None
    budget_id: str
    spend: Amount
    metadata: dict[str, JsonValue] | None
    models: list[str]
    model_spend: dict[str, JsonValue] | None
    created_by: str
    updated_by: str
    users: list[UserDump] | None
    litellm_budget_table: BudgetDump | None
    object_permission: PermissionDump | None
    object_permission_id: str | None
    members: list[MembershipDump]
    teams: list[TeamDump]
    created_at: str
    updated_at: str


# free-form JSON (metadata, model budgets, a membership's user) stays JsonValue
RecordDump: TypeAlias = (
    DeleteTeamDump | AddTeamCallbackDump
    | TeamCallbackMetadataDump | MemberDump
    | MemberAddDump | TeamMemberAddDump
    | OrganizationMemberAddDump | TeamMemberDeleteDump
    | OrganizationMemberDeleteDump | TeamMemberUpdateDump
    | OrganizationMemberUpdateDump | OrgTableUpdateDump
    | OrgUpdateV2Dump | OrgTableWithMembersDump
)
