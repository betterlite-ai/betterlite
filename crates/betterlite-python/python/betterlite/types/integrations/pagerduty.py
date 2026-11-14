from datetime import datetime
from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


# ast-grep-ignore: no-long-names
class StandardLoggingUserAPIKeyMetadata(TypedDict):
    user_api_key_hash: str | None
    user_api_key_alias: str | None
    user_api_key_spend: float | None
    user_api_key_max_budget: float | None
    user_api_key_budget_reset_at: str | None
    user_api_key_user_spend: float | None
    user_api_key_user_max_budget: float | None
    user_api_key_team_spend: float | None
    user_api_key_team_max_budget: float | None
    user_api_key_org_id: str | None
    user_api_key_org_alias: str | None
    user_api_key_team_id: str | None
    user_api_key_project_id: str | None
    user_api_key_project_alias: str | None
    user_api_key_user_id: str | None
    user_api_key_user_email: str | None
    user_api_key_team_alias: str | None
    user_api_key_end_user_id: str | None
    user_api_key_request_route: str | None
    user_api_key_auth_metadata: dict[str, str] | None


class LinkDict(TypedDict, total=False):
    href: str
    text: str | None


class ImageDict(TypedDict, total=False):
    src: str
    href: str | None
    alt: str | None


class PagerDutyPayload(TypedDict, total=False):
    summary: str
    timestamp: str | None
    severity: Literal["critical", "warning", "error", "info"]
    source: str
    component: str | None
    group: str | None
    class_: str | None
    custom_details: dict[str, JsonValue] | None


class PagerDutyRequestBody(TypedDict, total=False):
    payload: PagerDutyPayload
    routing_key: str
    event_action: Literal["trigger", "acknowledge", "resolve"]
    dedup_key: str | None
    client: str | None
    client_url: str | None
    links: list[LinkDict] | None
    images: list[ImageDict] | None


class AlertingConfig(TypedDict, total=False):
    failure_threshold: int
    failure_threshold_window_seconds: int  # ast-grep-ignore: no-long-names
    hanging_threshold_seconds: float
    hanging_threshold_fails: int
    hanging_threshold_window_seconds: int  # ast-grep-ignore: no-long-names


# ast-grep-ignore: no-long-names
class PagerDutyInternalEvent(StandardLoggingUserAPIKeyMetadata, total=False):
    failure_event_type: Literal["failed_response", "hanging_response"]
    timestamp: datetime
    error_class: str | None
    error_code: str | None
    error_llm_provider: str | None
