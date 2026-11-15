from collections.abc import Mapping
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import TypedDict


class UserAPIKeyLabelValuesDump(TypedDict):
    end_user: str | None
    user: str | None
    user_email: str | None
    user_alias: str | None
    hashed_api_key: str | None
    api_key_alias: str | None
    team: str | None
    team_alias: str | None
    model_group: str | None
    requested_model: str | None
    model: str | None
    litellm_model_name: str | None
    tags: list[str]
    custom_metadata_labels: dict[str, str]
    model_id: str | None
    api_base: str | None
    api_provider: str | None
    exception_status: str | None
    exception_class: str | None
    rate_limit_category: str | None
    rate_limit_type: str | None
    status_code: str | None
    fallback_model: str | None
    route: str | None
    client_ip: str | None
    user_agent: str | None
    stream: str | None
    org_id: str | None
    org_alias: str | None
    mcp_tool_name: str | None
    mcp_server_name: str | None
    service_tier: str | None
    input_sequence_length: str | None


@dataclass(frozen=True, slots=True, kw_only=True)
class UserAPIKeyLabelValues:
    end_user: str | None = None
    user: str | None = None
    user_email: str | None = None
    user_alias: str | None = None
    hashed_api_key: str | None = None
    api_key_alias: str | None = None
    team: str | None = None
    team_alias: str | None = None
    model_group: str | None = None
    requested_model: str | None = None
    model: str | None = None
    litellm_model_name: str | None = None
    tags: tuple[str, ...] | list[str] = ()
    custom_metadata_labels: Mapping[str, str] = field(default_factory=dict[str, str])
    model_id: str | None = None
    api_base: str | None = None
    api_provider: str | None = None
    exception_status: str | None = None
    exception_class: str | None = None
    rate_limit_category: str | None = None
    rate_limit_type: str | None = None
    status_code: str | None = None
    fallback_model: str | None = None
    route: str | None = None
    client_ip: str | None = None
    user_agent: str | None = None
    stream: str | None = None
    org_id: str | None = None
    org_alias: str | None = None
    mcp_tool_name: str | None = None
    mcp_server_name: str | None = None
    service_tier: str | None = None
    input_sequence_length: str | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "tags", tuple[str, ...](self.tags))
        labels = dict(self.custom_metadata_labels)
        object.__setattr__(self, "custom_metadata_labels", MappingProxyType[str, str](labels))

    def model_dump(self) -> UserAPIKeyLabelValuesDump:
        return {
            "end_user": self.end_user,
            "user": self.user,
            "user_email": self.user_email,
            "user_alias": self.user_alias,
            "hashed_api_key": self.hashed_api_key,
            "api_key_alias": self.api_key_alias,
            "team": self.team,
            "team_alias": self.team_alias,
            "model_group": self.model_group,
            "requested_model": self.requested_model,
            "model": self.model,
            "litellm_model_name": self.litellm_model_name,
            "tags": list(self.tags),
            "custom_metadata_labels": dict(self.custom_metadata_labels),
            "model_id": self.model_id,
            "api_base": self.api_base,
            "api_provider": self.api_provider,
            "exception_status": self.exception_status,
            "exception_class": self.exception_class,
            "rate_limit_category": self.rate_limit_category,
            "rate_limit_type": self.rate_limit_type,
            "status_code": self.status_code,
            "fallback_model": self.fallback_model,
            "route": self.route,
            "client_ip": self.client_ip,
            "user_agent": self.user_agent,
            "stream": self.stream,
            "org_id": self.org_id,
            "org_alias": self.org_alias,
            "mcp_tool_name": self.mcp_tool_name,
            "mcp_server_name": self.mcp_server_name,
            "service_tier": self.service_tier,
            "input_sequence_length": self.input_sequence_length,
        }


__all__ = ["UserAPIKeyLabelValues", "UserAPIKeyLabelValuesDump"]
