from typing import Literal, TypeAlias, TypedDict

import litellm
from pydantic import JsonValue

litellm.suppress_debug_info = True


class AuthFields(TypedDict, total=False):
    api_key: str
    rpm_limit: int | None
    tpm_limit: int | None
    max_parallel_requests: int | None
    budget_throttle_pct: float | None
    user_id: str | None
    user_rpm_limit: int | None
    user_tpm_limit: int | None
    team_id: str | None
    team_rpm_limit: int | None
    team_tpm_limit: int | None
    team_member_rpm_limit: int | None
    team_member_tpm_limit: int | None
    end_user_id: str | None
    end_user_rpm_limit: int | None
    end_user_tpm_limit: int | None
    agent_id: str | None
    org_id: str | None
    organization_rpm_limit: int | None
    organization_tpm_limit: int | None
    metadata: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue]
    organization_metadata: dict[str, JsonValue]
    project_metadata: dict[str, JsonValue]


class AgentFields(TypedDict, total=False):
    agent_id: str
    agent_name: str
    agent_card_params: dict[str, JsonValue]
    rpm_limit: int | None
    tpm_limit: int | None
    session_rpm_limit: int | None
    session_tpm_limit: int | None


class ServerFields(TypedDict, total=False):
    managed_agent_policy: AgentFields
    invoked_agent_id: str
    invoked_agent_policy: AgentFields
    source_team_rpm_limits: dict[str, dict[str, int]]


class TagLimitRow(TypedDict):
    tag_name: str
    rpm_limit: int | None
    tpm_limit: int | None


class CreatePolicy(TypedDict):
    rpm_limit_type: str | None
    tpm_limit_type: str | None
    model_has_failures: bool


class DynamicFlagCall(TypedDict):
    function: Literal["dynamic_flag"]
    rpm_limit_type: str | None
    tpm_limit_type: str | None


class ShouldEnforceCall(TypedDict):
    function: Literal["should_enforce"]
    limit_type: str | None
    model_has_failures: bool


class EnforcedLimitCall(TypedDict):
    function: Literal["enforced_limit"]
    limit_value: int | None
    limit_type: str | None
    model_has_failures: bool


class DescriptorCase(TypedDict):
    auth: AuthFields
    server: ServerFields
    data: dict[str, JsonValue]
    groups: dict[str, str]
    registry: list[AgentFields]
    tag_limits: list[TagLimitRow]
    window_size: int
    call_type: str | None


class CreateCall(DescriptorCase):
    function: Literal["create"]
    rpm_limit_type: str | None
    tpm_limit_type: str | None
    model_has_failures: bool


class AssembleCall(DescriptorCase):
    function: Literal["assemble"]


Input: TypeAlias = (
    DynamicFlagCall | ShouldEnforceCall | EnforcedLimitCall | CreateCall | AssembleCall
)


class RateLimitView(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorView(TypedDict):
    key: str
    value: str
    rate_limit: RateLimitView | None


class DescriptorAnswer(TypedDict):
    descriptors: list[DescriptorView]


class DynamicAnswer(TypedDict):
    dynamic: bool


class EnforceAnswer(TypedDict):
    enforce: bool


class LimitAnswer(TypedDict):
    limit: int | None
