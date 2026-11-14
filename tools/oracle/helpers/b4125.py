import json
import os
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.integrations.anthropic_cache_control_hook import AnthropicCacheControlHook
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OPENAI_BASE_URL_VARS: Final = ("OPENAI_BASE_URL", "OPENAI_API_BASE")


class Injector(Protocol):
    def maybe_inject_cache_control(
        self,
        messages: list[dict[str, JsonValue]],
        system: str | list[dict[str, JsonValue]] | None,
        kwargs: dict[str, JsonValue],
        *,
        model: str | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
    ) -> tuple[list[dict[str, JsonValue]], str | list[dict[str, JsonValue]] | None]: ...


class SeedPoints(Protocol):
    def __call__(
        self,
        non_default_params: dict[str, JsonValue],
        messages: list[dict[str, JsonValue]],
        model: str,
        custom_llm_provider: str | None,
        **options: JsonValue,
    ) -> None: ...


class ProjectMessages(Protocol):
    def __call__(
        self,
        messages: list[dict[str, JsonValue]],
        models: list[str],
        **options: JsonValue,
    ) -> list[dict[str, JsonValue]]: ...


hook: Final = AnthropicCacheControlHook()
hook_injector: Injector = hook
seed_points: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/anthropic_cache_control_hook.py#L814
    SeedPoints, hook.maybe_seed_default_injection_points
)
project_messages: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/anthropic_cache_control_hook.py#L765
    ProjectMessages, hook.messages_with_default_injections
)


class Setup(TypedDict):
    model_cost: dict[str, JsonValue]
    global_api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    global_enable: NotRequired[bool]
    ttl: NotRequired[Literal["5m", "1h"] | None]
    alias_map: NotRequired[dict[str, str]]


class Probe(Setup):
    model: str
    messages: list[dict[str, JsonValue]]
    params: dict[str, JsonValue]
    system: NotRequired[str | list[dict[str, JsonValue]] | None]
    tools: NotRequired[list[dict[str, JsonValue]] | None]


class SeedingProbe(Setup):
    model: str
    messages: list[dict[str, JsonValue]]
    params: dict[str, JsonValue]
    tools: NotRequired[list[JsonValue] | None]
    enable_prompt_caching: NotRequired[bool | None]


class ProjectionProbe(Setup):
    models: list[str]
    messages: list[dict[str, JsonValue]]
    tools: NotRequired[list[JsonValue] | None]
    enable_prompt_caching: NotRequired[bool | None]
    request_kwargs: NotRequired[dict[str, JsonValue] | None]


class MessagesInput(TypedDict):
    operation: Literal["messages_injection"]
    probes: list[Probe]


class SeedingInput(TypedDict):
    operation: Literal["chat_seeding"]
    probes: list[SeedingProbe]


class ProjectionInput(TypedDict):
    operation: Literal["router_projection"]
    probes: list[ProjectionProbe]


Input: TypeAlias = MessagesInput | SeedingInput | ProjectionInput


def install(probe: Setup) -> None:
    litellm.suppress_debug_info = True
    litellm.enable_anthropic_prompt_caching = probe.get("global_enable") is True
    litellm.anthropic_prompt_caching_ttl = probe.get("ttl")
    litellm.api_base = probe.get("global_api_base")
    litellm.model_cost = probe["model_cost"]
    litellm.model_alias_map = {}
    alias_map = probe.get("alias_map")
    if alias_map is not None:
        litellm.model_alias_map = alias_map
    for name in OPENAI_BASE_URL_VARS:
        os.environ.pop(name, None)
    environment = probe.get("environment")
    if environment is not None:
        os.environ.update(environment)


def messages_injection(probe: Probe) -> JsonValue:
    install(probe)
    params = dict(probe["params"])
    messages, system = hook_injector.maybe_inject_cache_control(
        messages=probe["messages"],
        system=probe.get("system"),
        kwargs=params,
        model=probe["model"],
        tools=probe.get("tools"),
    )
    return {
        "messages": JSON.validate_json(json.dumps(messages)),
        "system": JSON.validate_json(json.dumps(system)),
        "params": JSON.validate_json(json.dumps(params)),
    }


def chat_seeding(probe: SeedingProbe) -> JsonValue:
    install(probe)
    params = dict(probe["params"])
    seed_points(
        non_default_params=params,
        messages=probe["messages"],
        model=probe["model"],
        custom_llm_provider=None,
        tools=probe.get("tools"),
        enable_prompt_caching=probe.get("enable_prompt_caching"),
    )
    return {"params": JSON.validate_json(json.dumps(params))}


def router_projection(probe: ProjectionProbe) -> JsonValue:
    install(probe)
    projected = project_messages(
        probe["messages"],
        probe["models"],
        tools=probe.get("tools"),
        enable_prompt_caching=probe.get("enable_prompt_caching"),
        request_kwargs=probe.get("request_kwargs"),
    )
    return {"messages": JSON.validate_json(json.dumps(projected))}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "messages_injection":
        return {"probes": [messages_injection(probe) for probe in input_value["probes"]]}
    if input_value["operation"] == "chat_seeding":
        return {"probes": [chat_seeding(probe) for probe in input_value["probes"]]}
    return {"probes": [router_projection(probe) for probe in input_value["probes"]]}
