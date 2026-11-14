from datetime import datetime
from typing import ClassVar, Final, Literal, TypeAlias

from pydantic import BaseModel, ConfigDict, Field, JsonValue, model_validator

from betterlite.types.llms.base import LiteLLMPydanticObjectBase

TeamIdSearchMatch: TypeAlias = Literal["exact", "prefix"]

MAX_BULK_TEAM_MEMBER_DELETES: Final[int] = 500

# ast-grep-ignore: no-long-names
MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES: Final[int] = 500


# ast-grep-ignore: no-long-names
class GetTeamMemberPermissionsRequest(BaseModel):
    team_id: str


# ast-grep-ignore: no-long-names
class GetTeamMemberPermissionsResponse(BaseModel):
    team_id: str
    team_member_permissions: list[str] | None = []
    all_available_permissions: list[str]


# ast-grep-ignore: no-long-names
class UpdateTeamMemberPermissionsRequest(BaseModel):
    team_id: str
    team_member_permissions: list[str]


# ast-grep-ignore: no-long-names
class BulkUpdateTeamMemberPermissionsRequest(BaseModel):
    permissions: list[str]
    team_ids: list[str] | None = None
    apply_to_all_teams: bool = False


# ast-grep-ignore: no-long-names
class BulkUpdateTeamMemberPermissionsResponse(BaseModel):
    message: str
    teams_updated: int
    permissions_appended: list[str] | None = None


class TeamListItem(LiteLLMPydanticObjectBase):
    team_alias: str | None = None
    team_id: str
    organization_id: str | None = None
    admins: list[str] = Field(default=[])
    members: list[str] = Field(default=[])
    members_with_roles: list[dict[str, JsonValue]] = Field(default=[])
    team_member_permissions: list[str] | None = None
    metadata: dict[str, JsonValue] | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    tpd_limit: int | None = None
    max_budget: float | None = None
    soft_budget: float | None = None
    budget_duration: str | None = None
    budget_limits: list[dict[str, JsonValue]] | None = None
    models: list[str] = Field(default=[])
    blocked: bool = False
    router_settings: dict[str, JsonValue] | None = None
    access_group_ids: list[str] | None = None
    default_team_member_models: list[str] | None = None
    spend: float | None = None
    max_parallel_requests: int | None = None
    budget_reset_at: datetime | None = None
    model_id: int | None = None
    model_spend: dict[str, JsonValue] | None = Field(default={})
    model_max_budget: dict[str, JsonValue] | None = Field(default={})
    policies: list[str] | None = None
    allow_team_guardrail_config: bool | None = False
    litellm_model_table: dict[str, JsonValue] | None = None
    object_permission: dict[str, JsonValue] | None = None
    object_permission_id: str | None = None
    updated_at: datetime | None = None
    created_at: datetime | None = None
    members_count: int = 0
    keys_count: int = 0
    access_group_models: list[str] | None = None
    access_group_mcp_server_ids: list[str] | None = None
    access_group_agent_ids: list[str] | None = None


class TeamListResponse(BaseModel):
    teams: list[TeamListItem]
    total: int
    page: int
    page_size: int
    total_pages: int


class BulkTeamMemberAddRequest(BaseModel):
    team_id: str
    members: list[dict[str, JsonValue]] | None = None
    all_users: bool | None = False
    max_budget_in_team: float | None = None


class TeamMemberAddResult(BaseModel):
    user_id: str | None = None
    user_email: str | None = None
    success: bool
    error: str | None = None
    updated_user: dict[str, JsonValue] | None = None
    updated_team_membership: dict[str, JsonValue] | None = None


class BulkTeamMemberAddResponse(BaseModel):
    team_id: str
    results: list[TeamMemberAddResult]
    total_requested: int
    successful_additions: int
    failed_additions: int
    updated_team: dict[str, JsonValue] | None = None


class TeamMemberRef(LiteLLMPydanticObjectBase):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    user_id: str | None = None
    user_email: str | None = None

    @model_validator(mode="before")
    @classmethod
    def check_user_info(cls, values: dict[str, JsonValue]) -> dict[str, JsonValue]:
        if values.get("user_id") is None and values.get("user_email") is None:
            raise ValueError("Either user id or user email must be provided")
        return values

    @model_validator(mode="after")
    def one_identifier(self) -> "TeamMemberRef":
        if self.user_id is not None and self.user_email is not None:
            raise ValueError(
                "Each member must be identified by exactly one of user_id or user_email"
            )
        return self


class BulkTeamMemberDeleteRequest(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    members: tuple[TeamMemberRef, ...] = Field(
        min_length=1, max_length=MAX_BULK_TEAM_MEMBER_DELETES
    )


class TeamMemberDeleteResult(BaseModel):
    user_id: str | None = None
    user_email: str | None = None
    success: bool
    error: str | None = None


class BulkTeamMemberDeleteResponse(BaseModel):
    data: tuple[TeamMemberDeleteResult, ...]


class TeamMemberBudgetPatch(TeamMemberRef):
    max_budget_in_team: float | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    budget_duration: str | None = None
    allowed_models: tuple[str, ...] | None = None


# ast-grep-ignore: no-long-names
class BulkTeamMemberBudgetUpdateRequest(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    members: tuple[TeamMemberBudgetPatch, ...] = Field(
        min_length=1, max_length=MAX_BULK_TEAM_MEMBER_BUDGET_UPDATES
    )


class TeamMemberBudgetUpdateResult(BaseModel):
    user_id: str | None = None
    user_email: str | None = None
    success: bool
    error: str | None = None
    budget_id: str | None = None
    max_budget: float | None = None
    max_budget_source: Literal["member", "team_default"] | None = None
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    budget_duration: str | None = None
    allowed_models: tuple[str, ...] | None = None


# ast-grep-ignore: no-long-names
class BulkTeamMemberBudgetUpdateResponse(BaseModel):
    data: tuple[TeamMemberBudgetUpdateResult, ...]


class TeamMemberInfoResponse(LiteLLMPydanticObjectBase):
    user_id: str
    team_id: str
    budget_id: str | None = None
    spend: float | None = 0.0
    total_spend: float | None = 0.0
    litellm_budget_table: dict[str, JsonValue] | None = None
    role: str | None = None
    user_email: str | None = None
    team_alias: str | None = None


class TeamMetadataFieldSchema(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    key: str = Field(min_length=1)
    label: str | None = None


class TeamMetadataSchemaResponse(BaseModel):
    fields: tuple[TeamMetadataFieldSchema, ...]


class TeamUserSpendRow(BaseModel):
    team_id: str
    team_alias: str | None = None
    user_id: str
    user_email: str | None = None
    user_alias: str | None = None
    spend: float = 0.0
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0
    api_requests: int = 0
    successful_requests: int = 0
    failed_requests: int = 0


class TeamUserSpendResponse(BaseModel):
    start_date: str
    end_date: str
    results: tuple[TeamUserSpendRow, ...]
