from collections.abc import Callable, Mapping, Sequence
from typing import Literal, NotRequired, Protocol, TypedDict, cast

from litellm.proxy import proxy_server
from litellm.proxy.agent_endpoints import agent_registry
from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from litellm.proxy.hooks.rate_limiter_utils import resolve_llm_provider_for_rate_limit
from litellm.router import Router
from litellm.types.agents import AgentResponse
from pydantic import JsonValue, TypeAdapter


class Registry(Protocol):
    def load_agents_from_config(
        self, agent_config: Sequence[Mapping[str, JsonValue]] | None = None
    ) -> None: ...

    def register_agent(self, agent_config: AgentResponse) -> None: ...

    def get_agent_by_id(self, agent_id: str) -> AgentResponse | None: ...


class ErrorFields(Protocol):
    status_code: int
    headers: dict[str, str] | None
    message: str
    category: str | None
    rate_limit_type: str | None
    model: str
    llm_provider: str
    detail: JsonValue


class AgentLookupInput(TypedDict):
    config_agents: list[dict[str, JsonValue]]
    registered_agents: list[dict[str, JsonValue]]
    lookup_id: str


class AgentSnapshot(TypedDict):
    agent_id: str
    litellm_params: dict[str, JsonValue]


class AgentLookupOutput(TypedDict):
    found: AgentSnapshot | None


class ProviderResolutionInput(TypedDict):
    model: str | None
    router: Literal["absent", "empty", "deployments"]
    deployments: list[dict[str, JsonValue]]


class ProviderPair(TypedDict):
    model: str
    provider: str


class RateLimitErrorInput(TypedDict):
    detail: JsonValue
    headers: dict[str, JsonValue] | None
    category: str | None
    rate_limit_type: str | None
    model: str | None
    llm_provider: str | None


class RateLimitErrorOutput(TypedDict):
    status_code: int
    headers: dict[str, str] | None
    message: str
    category: str | None
    rate_limit_type: str | None
    model: str
    llm_provider: str
    detail: JsonValue


class Input(TypedDict):
    agent_lookup: NotRequired[AgentLookupInput]
    provider_resolution: NotRequired[ProviderResolutionInput]
    rate_limit_error: NotRequired[RateLimitErrorInput]


REGISTRY: Callable[[], Registry] = TypeAdapter(Callable[[], Registry]).validate_python(
    agent_registry.AgentRegistry
)
INPUT: TypeAdapter[Input] = TypeAdapter(Input)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
JSON_OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


def agent_snapshot(agent: AgentResponse) -> AgentSnapshot:
    params = agent.litellm_params
    if params is None:
        return {"agent_id": agent.agent_id, "litellm_params": {}}
    return {
        "agent_id": agent.agent_id,
        "litellm_params": JSON_OBJECT.validate_python(params),
    }


def run_agent_lookup(call: AgentLookupInput) -> AgentLookupOutput:
    registry = REGISTRY()
    for record in call["registered_agents"]:
        registry.register_agent(AgentResponse.model_validate(record))
    registry.load_agents_from_config(call["config_agents"])
    found = registry.get_agent_by_id(call["lookup_id"])
    if found is None:
        return {"found": None}
    return {"found": agent_snapshot(found)}


def fixture_router(call: ProviderResolutionInput) -> Router | None:
    kind = call["router"]
    if kind == "absent":
        return None
    if kind == "empty":
        return Router(model_list=[])
    return Router(model_list=call["deployments"])


def run_provider_resolution(call: ProviderResolutionInput) -> ProviderPair:
    proxy_server.llm_router = fixture_router(call)
    model, provider = resolve_llm_provider_for_rate_limit(model=call["model"])
    return {"model": model, "provider": provider}


def build_error(call: RateLimitErrorInput) -> ErrorFields:
    detail = call["detail"]
    headers = call["headers"]
    rate_limit_type = call["rate_limit_type"]
    model = call["model"]
    llm_provider = call["llm_provider"]
    category = call["category"]
    if category is None:
        error = ProxyRateLimitError(
            detail=detail,
            headers=headers,
            rate_limit_type=rate_limit_type,
            model=model,
            llm_provider=llm_provider,
        )
    else:
        error = ProxyRateLimitError(
            detail=detail,
            headers=headers,
            category=category,
            rate_limit_type=rate_limit_type,
            model=model,
            llm_provider=llm_provider,
        )
    return cast(ErrorFields, error)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/proxy_rate_limit_error.py#L192


def run_rate_limit_error(call: RateLimitErrorInput) -> RateLimitErrorOutput:
    error = build_error(call)
    return {
        "status_code": error.status_code,
        "headers": error.headers,
        "message": error.message.removeprefix("litellm.RateLimitError: "),
        "category": error.category,
        "rate_limit_type": error.rate_limit_type,
        "model": error.model,
        "llm_provider": error.llm_provider,
        "detail": error.detail,
    }


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    agent_call = parsed.get("agent_lookup")
    if agent_call is not None:
        return JSON.validate_python(run_agent_lookup(agent_call))
    provider_call = parsed.get("provider_resolution")
    if provider_call is not None:
        return JSON.validate_python(run_provider_resolution(provider_call))
    error_call = parsed.get("rate_limit_error")
    if error_call is not None:
        return JSON.validate_python(run_rate_limit_error(error_call))
    raise ValueError("send one of agent_lookup, provider_resolution or rate_limit_error")
