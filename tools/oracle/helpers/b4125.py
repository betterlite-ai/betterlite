import json
import os
from typing import Final, Literal, NotRequired, Protocol, TypedDict

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


hook: Injector
hook = AnthropicCacheControlHook()


class Probe(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    params: dict[str, JsonValue]
    system: NotRequired[str | list[dict[str, JsonValue]] | None]
    tools: NotRequired[list[dict[str, JsonValue]] | None]
    global_api_base: NotRequired[str | None]
    environment: NotRequired[dict[str, str]]
    global_enable: NotRequired[bool]
    ttl: NotRequired[Literal["5m", "1h"] | None]
    model_cost: dict[str, JsonValue]


class Input(TypedDict):
    operation: Literal["messages_injection"]
    probes: list[Probe]


def install(probe: Probe) -> None:
    litellm.enable_anthropic_prompt_caching = probe.get("global_enable") is True
    litellm.anthropic_prompt_caching_ttl = probe.get("ttl")
    litellm.api_base = probe.get("global_api_base")
    litellm.model_cost = probe["model_cost"]
    for name in OPENAI_BASE_URL_VARS:
        os.environ.pop(name, None)
    environment = probe.get("environment")
    if environment is not None:
        os.environ.update(environment)


def messages_injection(probe: Probe) -> JsonValue:
    install(probe)
    params = dict(probe["params"])
    messages, system = hook.maybe_inject_cache_control(
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


def run(input_value: Input) -> JsonValue:
    results = [messages_injection(probe) for probe in input_value["probes"]]
    return {"probes": results}
