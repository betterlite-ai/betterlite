import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, TypeVar, cast

import litellm
from litellm import main as litellm_main
from litellm import utils as litellm_utils
from litellm.llms.anthropic.pass_through.adapters import handler as adapter_handler
from litellm.llms.anthropic.pass_through.adapters.handler import (
    LiteLLMMessagesToCompletionTransformationHandler as MessagesHandler,
)
from pydantic import JsonValue

ENV: Final = "LITELLM_REASONING_AUTO_SUMMARY"


class ResolvedScript(TypedDict):
    local: str
    provider: str
    api_base: str | None


class BridgeScript(TypedDict):
    mode: str | None
    fails: bool


class NormalizeCall(TypedDict):
    function: Literal["normalize_effort"]
    kwargs: dict[str, JsonValue]
    model_info: dict[str, JsonValue] | None


class RouteCall(TypedDict):
    function: Literal["route"]
    kwargs: dict[str, JsonValue]
    thinking: JsonValue
    model_info: dict[str, JsonValue] | None
    provider: ResolvedScript | None
    global_value: bool | None
    environment: str | None


class PlainCall(TypedDict):
    function: Literal["plain_effort"]
    kwargs: dict[str, JsonValue]
    thinking: JsonValue
    provider: ResolvedScript | None
    bridge: BridgeScript


Call: TypeAlias = NormalizeCall | RouteCall | PlainCall


class Input(TypedDict):
    calls: list[Call]


class Normalizer(Protocol):
    def __call__(self, completion_kwargs: dict[str, JsonValue], /) -> None: ...


class Router(Protocol):
    def __call__(
        self, completion_kwargs: dict[str, JsonValue], *, thinking: JsonValue
    ) -> None: ...


class PlainTarget(Protocol):
    def __call__(
        self, completion_kwargs: dict[str, JsonValue], *, thinking: JsonValue
    ) -> str | None: ...


NORMALIZE: Final = cast(Normalizer, MessagesHandler._normalize_reasoning_effort)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L431
ROUTE: Final = cast(Router, MessagesHandler._route_openai_thinking_to_responses_api_if_needed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L323
PLAIN: Final = cast(PlainTarget, MessagesHandler._plain_effort_for_chat_target)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L388


ScriptT = TypeVar("ScriptT")


def provided(script: ScriptT | None, failure: str) -> ScriptT:
    if script is None:
        raise RuntimeError(failure)
    return script


class ModelInfoFake:
    def __init__(
        self, *, script: dict[str, JsonValue] | None, record: list[JsonValue]
    ) -> None:
        self.script = script
        self.record = record

    def __call__(
        self, *, model: str | None, custom_llm_provider: str | None = None
    ) -> dict[str, JsonValue]:
        self.record.append(
            {
                "seam": "model_info",
                "args": {"model": model, "custom_llm_provider": custom_llm_provider},
            }
        )
        return provided(self.script, "the model lookup fails")


class ProviderFake:
    def __init__(self, *, script: ResolvedScript | None, record: list[JsonValue]) -> None:
        self.script = script
        self.record = record

    def __call__(
        self,
        *,
        model: str | None,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> tuple[str, str, str | None, str | None]:
        self.record.append(
            {
                "seam": "provider",
                "args": {
                    "model": model,
                    "custom_llm_provider": custom_llm_provider,
                    "api_base": api_base,
                    "api_key": api_key,
                },
            }
        )
        script = provided(self.script, "the provider lookup fails")
        return (script["local"], script["provider"], None, script["api_base"])


class BridgeFake:
    def __init__(self, *, script: BridgeScript, record: list[JsonValue]) -> None:
        self.script = script
        self.record = record

    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1083-L1091
        self,
        *,
        model: str,
        custom_llm_provider: str,
        web_search_options: JsonValue,
        tools: JsonValue,
        reasoning_effort: JsonValue,
        api_base: JsonValue,
    ) -> tuple[dict[str, JsonValue], str]:
        self.record.append(
            {
                "seam": "bridge",
                "args": {
                    "model": model,
                    "custom_llm_provider": custom_llm_provider,
                    "web_search_options": web_search_options,
                    "tools": tools,
                    "reasoning_effort": reasoning_effort,
                    "api_base": api_base,
                },
            }
        )
        if self.script["fails"]:
            raise RuntimeError("the bridge policy fails")
        mode = self.script["mode"]
        info: dict[str, JsonValue] = {}
        if mode is not None:
            info["mode"] = mode
        return (info, model)


def set_environment(value: str | None) -> None:
    if value is None:
        os.environ.pop(ENV, None)
    else:
        os.environ[ENV] = value


def normalize_answer(call: NormalizeCall) -> JsonValue:
    record: list[JsonValue] = []
    litellm_utils.get_model_info = ModelInfoFake(script=call["model_info"], record=record)  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/utils.py#L92
    kwargs = dict(call["kwargs"])
    NORMALIZE(kwargs)
    return {"kwargs": kwargs, "seams": record}


def route_answer(call: RouteCall) -> JsonValue:
    record: list[JsonValue] = []
    litellm_utils.get_llm_provider = ProviderFake(script=call["provider"], record=record)  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L342
    adapter_handler.get_model_info = ModelInfoFake(script=call["model_info"], record=record)  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L357
    kwargs = dict(call["kwargs"])
    previous = os.environ.get(ENV)
    set_environment(call["environment"])
    try:
        litellm.reasoning_auto_summary = call["global_value"] is True
        ROUTE(kwargs, thinking=call["thinking"])
        return {"kwargs": kwargs, "seams": record}
    finally:
        set_environment(previous)


def plain_answer(call: PlainCall) -> JsonValue:
    record: list[JsonValue] = []
    litellm_utils.get_llm_provider = ProviderFake(script=call["provider"], record=record)  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L404
    litellm_main.responses_api_bridge_check = BridgeFake(script=call["bridge"], record=record)  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L418
    kwargs = dict(call["kwargs"])
    effort = PLAIN(kwargs, thinking=call["thinking"])
    return {"effort": effort, "seams": record}


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "normalize_effort":
            return normalize_answer(call)
        case "route":
            return route_answer(call)
        case "plain_effort":
            return plain_answer(call)


def run(input_value: Input) -> JsonValue:
    answers = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
