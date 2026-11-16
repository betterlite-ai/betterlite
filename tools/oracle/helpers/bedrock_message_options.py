from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig as MessagesConfig,
)
from pydantic import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]


class TtlCase(TypedDict):
    model: str | None
    request: JsonObject


class EnsureCase(TypedDict):
    model: str
    request: JsonObject


class ClampCase(TypedDict):
    model: str
    params: JsonObject


class RemoveTtlCall(TypedDict):
    function: Literal["remove_ttl"]
    model_cost: JsonObject
    cases: list[TtlCase]


class EnsureThinkingCall(TypedDict):
    function: Literal["ensure_thinking"]
    model_cost: JsonObject
    cases: list[EnsureCase]


class ExtendedThinkingCall(TypedDict):
    function: Literal["supports_extended_thinking"]
    model_cost: JsonObject
    models: list[str]


class ClampEffortCall(TypedDict):
    function: Literal["clamp_effort"]
    model_cost: JsonObject
    cases: list[ClampCase]


Input = RemoveTtlCall | EnsureThinkingCall | ExtendedThinkingCall | ClampEffortCall


class Config(MessagesConfig):
    def remove_ttl(self, request: JsonObject, model: str | None) -> None:
        self._remove_ttl_from_cache_control(  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/messages/invoke_transformations/anthropic_claude3_transformation.py#L158
            request, model
        )

    def thinking_supported(self, model: str) -> bool:
        return self._supports_extended_thinking_on_bedrock(model)

    def ensure_thinking(self, request: JsonObject, model: str) -> bool:
        return self._ensure_thinking_for_clear_thinking_context_management(  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/messages/invoke_transformations/anthropic_claude3_transformation.py#L250
            request, model
        )

    def clamp_effort(self, model: str, params: JsonObject) -> None:
        self._clamp_adaptive_reasoning_effort_for_bedrock(  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/messages/invoke_transformations/anthropic_claude3_transformation.py#L657
            model=model,
            optional_params=params,
        )


CONFIG: Final = Config()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/messages/invoke_transformations/anthropic_claude3_transformation.py#L99-L101


def ttl_request(case: TtlCase) -> JsonValue:
    try:
        CONFIG.remove_ttl(case["request"], case["model"])
    except (AttributeError, TypeError):
        return {"raised": True}
    return case["request"]


def remove_ttl(call: RemoveTtlCall) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    requests: list[JsonValue] = [ttl_request(case) for case in call["cases"]]
    return {"requests": requests}


def thinking_request(case: EnsureCase) -> JsonValue:
    try:
        injected = CONFIG.ensure_thinking(case["request"], case["model"])
    except (AttributeError, TypeError, ValueError):
        return {"raised": True}
    return {"injected": injected, "request": case["request"]}


def ensure_thinking(call: EnsureThinkingCall) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = [thinking_request(case) for case in call["cases"]]
    return {"results": results}


def extended_thinking(call: ExtendedThinkingCall) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = [
        CONFIG.thinking_supported(model) for model in call["models"]
    ]
    return {"results": results}


def clamp_effort(call: ClampEffortCall) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    results: list[JsonValue] = []
    for case in call["cases"]:
        CONFIG.clamp_effort(case["model"], case["params"])
        results.append(case["params"])
    return {"results": results}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "remove_ttl":
        return remove_ttl(input_value)
    if input_value["function"] == "ensure_thinking":
        return ensure_thinking(input_value)
    if input_value["function"] == "supports_extended_thinking":
        return extended_thinking(input_value)
    return clamp_effort(input_value)
