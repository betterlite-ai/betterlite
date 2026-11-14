from collections.abc import Mapping, Sequence
from typing import Annotated, Final, Literal, Self, TypedDict

from pydantic import ConfigDict, Field
from typing_extensions import ReadOnly

from betterlite.types.proxy.management_endpoints.management_v1 import ResourceResponse
from betterlite.types.utils import (
    JsonValue,
    ResponseObject,
    field_validator,
    model_validator,
)

MAX_BULK_DELETE_USERS: Final = 500

MAX_BULK_NEW_USERS: Final = 500


class InsensitiveContains(TypedDict):
    contains: ReadOnly[str]
    mode: ReadOnly[Literal["insensitive"]]


class UserSearchWhere(TypedDict):
    OR: ReadOnly[
        tuple[Mapping[Literal["user_id", "user_email"], InsensitiveContains], ...]
    ]


class UserListResponse(ResponseObject):
    users: list[dict[str, JsonValue]]
    total: int
    page: int
    page_size: int
    total_pages: int


class BulkUpdateUserRequest(ResponseObject):
    users: list[dict[str, JsonValue]] | None = None
    all_users: bool | None = False
    user_updates: dict[str, JsonValue] | None = None

    @model_validator(mode="after")
    def validate_request(self) -> Self:
        if "user_updates" not in self.model_fields_set:
            return self
        has_users = self.users is not None and len(self.users) > 0
        has_updates = self.user_updates is not None
        if not has_users and not (self.all_users is True and has_updates):
            raise ValueError(
                "Must specify either 'users' for individual updates or "
                "'all_users=True' with 'user_updates' for bulk updates"
            )
        if has_users and self.all_users is True:
            raise ValueError(
                "Cannot specify both 'users' and 'all_users=True'. Choose one approach."
            )
        return self


class UserUpdateResult(ResponseObject):
    user_id: str | None = None
    user_email: str | None = None
    success: bool
    error: str | None = None
    updated_user: dict[str, JsonValue] | None = None


class BulkUpdateUserResponse(ResponseObject):
    results: list[UserUpdateResult]
    total_requested: int
    successful_updates: int
    failed_updates: int


class BulkDeleteUserRequest(ResponseObject):
    model_config = ConfigDict(extra="forbid")

    user_ids: Annotated[
        tuple[str, ...],
        Field(min_length=1, max_length=MAX_BULK_DELETE_USERS),
    ]


class UserDeleteResult(ResponseObject):
    user_id: str
    user_email: str | None = None
    success: bool
    teams_removed: tuple[str, ...] = ()
    error: str | None = None


class BulkDeleteUsersResponse(ResourceResponse[tuple[UserDeleteResult, ...]]):
    pass


class BulkNewUserItem(ResponseObject):
    model_config = ConfigDict(extra="forbid", protected_namespaces=())

    key_alias: str | None = None
    duration: str | None = None
    models: list[JsonValue] | None = Field(default_factory=list)
    spend: float | None = 0
    max_budget: float | None = None
    user_id: str | None = None
    team_id: str | None = None
    agent_id: str | None = None
    max_parallel_requests: int | None = None
    metadata: dict[str, JsonValue] | None = Field(default_factory=dict)
    tpm_limit: int | None = None
    rpm_limit: int | None = None
    budget_duration: str | None = None
    budget_limits: list[dict[str, JsonValue]] | None = None
    allowed_cache_controls: list[JsonValue] | None = Field(default_factory=list)
    config: dict[str, JsonValue] | None = Field(default_factory=dict)
    permissions: dict[str, JsonValue] | None = Field(default_factory=dict)
    model_max_budget: dict[str, JsonValue] | None = Field(default_factory=dict)
    budget_fallbacks: dict[str, list[str]] | None = None
    model_rpm_limit: dict[str, JsonValue] | None = None
    model_tpm_limit: dict[str, JsonValue] | None = None
    mcp_rpm_limit: dict[str, int] | None = None
    tag_rpm_limit: dict[str, int] | None = None
    guardrails: list[str] | None = None
    policies: list[str] | None = None
    prompts: list[str] | None = None
    blocked: bool | None = None
    aliases: dict[str, JsonValue] | None = Field(default_factory=dict)
    object_permission: dict[str, JsonValue] | None = None
    user_email: str | None = None
    user_alias: str | None = None
    user_role: (
        Literal[
            "proxy_admin",
            "proxy_admin_viewer",
            "internal_user",
            "internal_user_viewer",
        ]
        | None
    ) = None
    teams: list[str] | list[dict[str, JsonValue]] | None = None
    auto_create_key: bool = False
    send_invite_email: bool | None = None
    sso_user_id: str | None = None
    organizations: list[str] | None = None
    password: str | None = None

    @field_validator("send_invite_email")
    @classmethod
    def reject_invite_email(cls, value: JsonValue) -> JsonValue:
        if value is True:
            raise ValueError(
                "send_invite_email is not supported on /management/v1/users/bulk; "
                "invite users separately"
            )
        return value

    @field_validator("password")
    @classmethod
    def reject_password(cls, value: JsonValue) -> JsonValue:
        if value is not None:
            raise ValueError(
                "password cannot be set via /user/new. Users set their own password "
                "through an invitation link (POST /invitation/new)."
            )
        return value


class BulkNewUserRequest(ResponseObject):
    model_config = ConfigDict(extra="forbid")

    users: Annotated[
        Sequence[BulkNewUserItem],
        Field(min_length=1, max_length=MAX_BULK_NEW_USERS),
    ]


class UserCreateResult(ResponseObject):
    user_id: str | None = None
    user_email: str | None = None
    success: bool
    teams: tuple[str, ...] | None = None
    key: str | None = None
    error: str | None = None


class BulkNewUserMeta(ResponseObject):
    total_requested: int
    created: int
    failed: int


class BulkNewUserResponse(ResponseObject):
    data: tuple[UserCreateResult, ...]
    meta: BulkNewUserMeta
