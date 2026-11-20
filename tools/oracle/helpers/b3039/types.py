from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy.hooks.parallel_request_limiter_v3 import RateLimitDescriptor
from pydantic import JsonValue, TypeAdapter


class AuthFields(TypedDict):
    api_key: NotRequired[str]
    team_id: NotRequired[str]
    agent_id: NotRequired[str]
    metadata: NotRequired[dict[str, JsonValue]]
    team_metadata: NotRequired[dict[str, JsonValue]]


class AgentFields(TypedDict):
    agent_id: NotRequired[str]
    agent_name: NotRequired[str]
    agent_card_params: NotRequired[dict[str, JsonValue]]
    rpm_limit: NotRequired[int | None]
    tpm_limit: NotRequired[int | None]
    session_rpm_limit: NotRequired[int | None]
    session_tpm_limit: NotRequired[int | None]


class TagDescriptorCall(TypedDict):
    function: Literal["tag_descriptor"]
    tag: str
    rpm_limit: int | None
    tpm_limit: int | None
    window_size: int


class TagLimitRow(TypedDict):
    tag_name: str
    rpm_limit: int | None
    tpm_limit: int | None


class TagCreateCall(TypedDict):
    function: Literal["tag_create"]
    data: dict[str, JsonValue]
    tag_limits: list[TagLimitRow]
    window_size: int


class TagLimitFields(TypedDict):
    rpm_limit: int | None
    tpm_limit: int | None


class TagBudgetFixture(TypedDict):
    tag_name: str
    budget: TagLimitFields | None


class TagResolutionCall(TypedDict):
    function: Literal["tag_resolution"]
    tag_names: list[str]
    database: bool
    cached: list[TagBudgetFixture]
    rows: list[TagBudgetFixture]


class TagPerKeyCall(TypedDict):
    function: Literal["tag_per_key"]
    auth: AuthFields
    data: dict[str, JsonValue]
    window_size: int


class McpPerKeyCall(TypedDict):
    function: Literal["mcp_per_key"]
    auth: AuthFields
    mcp_server_name: str | None
    window_size: int


class McpPerTeamCall(TypedDict):
    function: Literal["mcp_per_team"]
    auth: AuthFields
    mcp_server_name: str | None
    source_team_rpm_limits: dict[str, dict[str, int]] | None
    window_size: int


class AgentCall(TypedDict):
    function: Literal["agent_descriptors"]
    agent_id: str
    data: dict[str, JsonValue]
    window_size: int
    policy: AgentFields | None
    registry: list[AgentFields]


class AgentResolutionCall(TypedDict):
    function: Literal["agent_resolution"]
    key_agent_id: str | None
    data: dict[str, JsonValue]


class SessionCall(TypedDict):
    function: Literal["session_resolution"]
    data: dict[str, JsonValue]


LimiterCall: TypeAlias = (
    TagDescriptorCall
    | TagCreateCall
    | TagPerKeyCall
    | McpPerKeyCall
    | McpPerTeamCall
    | AgentCall
)

Input: TypeAlias = LimiterCall | TagResolutionCall | AgentResolutionCall | SessionCall


class RateLimitView(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorView(TypedDict):
    key: str
    value: str
    rate_limit: RateLimitView | None


def descriptor_view(descriptor: RateLimitDescriptor) -> DescriptorView:
    rate_limit = descriptor["rate_limit"]
    if rate_limit is None:
        return DescriptorView(
            key=descriptor["key"], value=descriptor["value"], rate_limit=None
        )
    return DescriptorView(
        key=descriptor["key"],
        value=descriptor["value"],
        rate_limit=RateLimitView(
            requests_per_unit=rate_limit.get("requests_per_unit"),
            tokens_per_unit=rate_limit.get("tokens_per_unit"),
            max_parallel_requests=rate_limit.get("max_parallel_requests"),
            window_size=rate_limit.get("window_size"),
        ),
    )


class DescriptorsAnswer(TypedDict):
    descriptors: list[DescriptorView]


class TagCreateAnswer(TypedDict):
    descriptors: list[DescriptorView]
    resolver_calls: list[list[str]]


class LimitView(TypedDict):
    rpm_limit: int | None
    tpm_limit: int | None


class TagResolutionAnswer(TypedDict):
    limits: dict[str, LimitView]


class AgentAnswer(TypedDict):
    agent_id: str | None


class SessionAnswer(TypedDict):
    session_id: str | None


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
