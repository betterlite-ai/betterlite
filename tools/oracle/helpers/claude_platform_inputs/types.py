from typing import NotRequired, TypeAlias, TypedDict

import litellm.llms.bedrock.claude_platform.common_utils as platform

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Override: TypeAlias = Json | set[Json] | frozenset[Json] | tuple[Json, ...]

EMPTY_PARAMS: dict[str, Json] = {}
EMPTY_STRINGS: dict[str, str] = {}
EMPTY_HEADERS: list[tuple[str, str]] = []
EMPTY_HEADER_MAP: dict[str, str | None] = {}
OVERRIDE_KEY = platform.CLAUDE_PLATFORM_UNSUPPORTED_PARAMS_OVERRIDE_KEY
DROP_MARKER = "dropping unsupported Messages API param(s) "
IGNORE_MARKER = "ignoring claude_platform_unsupported_params of type "


class WorkspaceCase(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    deployment: NotRequired[dict[str, str]]
    litellm_settings: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class RegionCase(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class UrlCase(TypedDict):
    api_base: NotRequired[str | None]
    litellm_api_base: NotRequired[str | None]
    optional_params: NotRequired[dict[str, Json]]
    environment: NotRequired[dict[str, str]]


class OverrideCall(TypedDict):
    deployment: NotRequired[dict[str, Json]]
    optional: NotRequired[dict[str, Json]]
    log_invalid: NotRequired[bool]


class BodyCall(TypedDict):
    params: NotRequired[dict[str, Json]]
    unsupported: NotRequired[list[str]]
    log_dropped: NotRequired[bool]


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class SignCase(TypedDict):
    headers: NotRequired[dict[str, str | None]]
    optional_params: NotRequired[dict[str, Json]]
    request_data: NotRequired[dict[str, Json]]
    api_base: str
    api_key: NotRequired[str | None]
    model: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    credentials: Keys
    now: int


class ErrorClassCall(TypedDict):
    message: str
    status_code: int
    headers: NotRequired[list[tuple[str, str]]]


class SigningCall(TypedDict):
    sign: NotRequired[SignCase]
    error_class: NotRequired[ErrorClassCall]


class Input(TypedDict):
    workspace_alias_precedence: NotRequired[dict[str, list[WorkspaceCase]]]
    explicit_region_required: NotRequired[dict[str, list[RegionCase]]]
    api_base_before_region: NotRequired[dict[str, list[UrlCase]]]
    platform_route_prefix: NotRequired[dict[str, list[str]]]
    unsupported_override: NotRequired[dict[str, list[OverrideCall]]]
    filtered_body_fields: NotRequired[dict[str, list[BodyCall]]]
    platform_signing: NotRequired[dict[str, list[SigningCall]]]
