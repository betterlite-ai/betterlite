import json
from contextlib import suppress
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.proxy._experimental.mcp_server.sampling_handler import (  # pyright: ignore[reportPrivateUsage]
    _convert_mcp_messages_to_openai,  # pyright: ignore[reportPrivateUsage]
    _convert_mcp_tool_choice_to_openai,  # pyright: ignore[reportPrivateUsage]
    _convert_mcp_tools_to_openai,  # pyright: ignore[reportPrivateUsage]
    _convert_openai_response_to_mcp_result,  # pyright: ignore[reportPrivateUsage]
    _resolve_model_from_preferences,  # pyright: ignore[reportPrivateUsage]
)
from mcp.types import ModelHint, ModelPreferences, SamplingMessage, Tool, ToolChoice
from pydantic import BaseModel, ConfigDict, Field

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class HintInput(Strict):
    name: str | None = None


class PreferencesInput(Strict):
    hints: list[HintInput] | None = None
    cost_priority: float | None = Field(default=None, alias="costPriority")
    speed_priority: float | None = Field(default=None, alias="speedPriority")
    intelligence_priority: float | None = Field(default=None, alias="intelligencePriority")


class PriorityView(ModelPreferences):
    @property
    def costPriority(self) -> float | None:  # noqa: N802 - LiteLLM reads this camelCase attribute
        return self.cost_priority

    @property
    def speedPriority(self) -> float | None:  # noqa: N802 - LiteLLM reads this camelCase attribute
        return self.speed_priority

    @property
    def intelligencePriority(self) -> float | None:  # noqa: N802 - LiteLLM reads this camelCase attribute
        return self.intelligence_priority


class ModelInfoInput(Strict):
    input_cost_per_token: float | None = None
    output_cost_per_token: float | None = None
    max_output_tokens: int | None = None
    max_tokens: int | None = None
    output_tokens_per_second: float | None = None

    def python_info(self) -> dict[str, object]:
        return {
            "input_cost_per_token": self.input_cost_per_token,
            "output_cost_per_token": self.output_cost_per_token,
            "max_output_tokens": self.max_output_tokens,
            "max_tokens": self.max_tokens,
            "output_tokens_per_second": self.output_tokens_per_second,
        }


class ResolveScenario(Strict):
    available_models: list[str] = Field(default_factory=list)
    default_model: str | None = None
    fallback_model: str | None = None
    model_info: dict[str, ModelInfoInput] = Field(default_factory=dict)
    preferences: PreferencesInput | None = None


class ResolveInput(Strict):
    operation: str
    scenarios: list[ResolveScenario] = Field(default_factory=list)


class MessageScenario(Strict):
    system_prompt: str | None = None
    messages: list[dict[str, object]] = Field(default_factory=list)


class MessagesInput(Strict):
    operation: str
    scenarios: list[MessageScenario] = Field(default_factory=list)


class ToolScenario(Strict):
    tools: list[dict[str, object]] | None = None
    tool_choice: dict[str, object] | None = None


class ToolsInput(Strict):
    operation: str
    scenarios: list[ToolScenario] = Field(default_factory=list)


class ResponseScenario(Strict):
    model_name: str
    response: dict[str, object]


class ResponseInput(Strict):
    operation: str
    scenarios: list[ResponseScenario] = Field(default_factory=list)


def mcp_preferences(given: PreferencesInput) -> PriorityView:
    hints = None if given.hints is None else [ModelHint(name=hint.name) for hint in given.hints]
    return PriorityView(
        hints=hints,
        cost_priority=given.cost_priority,
        speed_priority=given.speed_priority,
        intelligence_priority=given.intelligence_priority,
    )


def resolve_scenario(scenario: ResolveScenario) -> Json:
    litellm.model_list = list(scenario.available_models)
    if scenario.fallback_model is None:
        with suppress(AttributeError):
            del litellm.default_mcp_sampling_model  # pyright: ignore[reportAttributeAccessIssue]
    else:
        litellm.default_mcp_sampling_model = scenario.fallback_model  # pyright: ignore[reportAttributeAccessIssue]
    info = {name: fields.python_info() for name, fields in scenario.model_info.items()}

    def lookup(name: str) -> dict[str, object]:
        return info[name]

    preferences = None if scenario.preferences is None else mcp_preferences(scenario.preferences)
    with patch("litellm.get_model_info", side_effect=lookup):
        try:
            model = _resolve_model_from_preferences(preferences, scenario.default_model)
        except Exception:  # noqa: BLE001 - a refused resolution is one comparable outcome
            return {"outcome": "failed"}
    return {"outcome": "resolved", "model": model}


def messages_value(scenario: MessageScenario) -> Json:
    messages = [SamplingMessage.model_validate(message) for message in scenario.messages]
    converted = _convert_mcp_messages_to_openai(messages, scenario.system_prompt)
    rendered = cast(Json, json.loads(json.dumps(converted)))
    normalise_arguments(rendered)
    return rendered


def tools_value(scenario: ToolScenario) -> Json:
    tools = (
        None if scenario.tools is None else [Tool.model_validate(tool) for tool in scenario.tools]
    )
    tool_choice = (
        None if scenario.tool_choice is None else ToolChoice.model_validate(scenario.tool_choice)
    )
    converted_tools = _convert_mcp_tools_to_openai(tools)
    converted_choice = _convert_mcp_tool_choice_to_openai(tool_choice)
    return {
        "tools": cast(Json, json.loads(json.dumps(converted_tools))),
        "tool_choice": converted_choice,
    }


def response_value(scenario: ResponseScenario) -> Json:
    response = litellm.ModelResponse(**scenario.response)
    convert = _convert_openai_response_to_mcp_result
    try:
        result = convert(response, scenario.model_name)  # pyright: ignore[reportArgumentType]
    except Exception:  # noqa: BLE001 - a refused conversion is one comparable outcome
        return {"outcome": "failed"}
    dumped = cast(BaseModel, result).model_dump(by_alias=True, exclude_none=True, mode="json")
    return {"outcome": "result", "value": cast(Json, dumped)}


def normalise_arguments(value: Json) -> None:
    if not isinstance(value, list):
        return
    for message in value:
        if not isinstance(message, dict):
            continue
        tool_calls = message.get("tool_calls")
        if not isinstance(tool_calls, list):
            continue
        for call in tool_calls:
            if not isinstance(call, dict):
                continue
            function = call.get("function")
            if not isinstance(function, dict):
                continue
            arguments = function.get("arguments")
            if isinstance(arguments, str):
                function["arguments"] = cast(Json, json.loads(arguments))


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    match operation:
        case "resolve":
            parsed = ResolveInput.model_validate(input_value)
            return {"results": [resolve_scenario(scenario) for scenario in parsed.scenarios]}
        case "messages":
            parsed = MessagesInput.model_validate(input_value)
            return {"results": [messages_value(scenario) for scenario in parsed.scenarios]}
        case "tools":
            parsed = ToolsInput.model_validate(input_value)
            return {"results": [tools_value(scenario) for scenario in parsed.scenarios]}
        case "response":
            parsed = ResponseInput.model_validate(input_value)
            return {"results": [response_value(scenario) for scenario in parsed.scenarios]}
        case _:
            raise ValueError(f"unknown b8192 operation: {operation}")
