import copy
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    _bedrock_converse_messages_pt,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _bedrock_tools_pt,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)
from litellm.llms.bedrock import common_utils as bedrock_common
from litellm.llms.bedrock.chat import converse_transformation

if TYPE_CHECKING:
    from litellm.types.llms.openai import (
        AllMessageValues,
        ChatCompletionAssistantMessage,
        ChatCompletionUserMessage,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

GENERATED_ID = "generated-id"


class _FixedUuid:
    def uuid4(self) -> str:
        return GENERATED_ID


class _Capabilities:
    def __init__(self, cache_points: str, *, supports_strict_tools: bool) -> None:
        self.cache_points = cache_points
        self.supports_strict_tools = supports_strict_tools

    def accepts_cache_points(self, _model: str | None) -> bool:
        return self.cache_points != "unsupported"

    def extended_ttl(self, _model: str | None) -> bool:
        return self.cache_points == "extended_ttl"

    def strict_tools(self, _model: str) -> bool:
        return self.supports_strict_tools


def _capabilities(input_value: dict[str, Json]) -> _Capabilities:
    raw = input_value.get("capabilities")
    capabilities = raw if isinstance(raw, dict) else {}
    cache_points = capabilities.get("cache_points")
    supports_strict_tools = capabilities.get("supports_strict_tools")
    return _Capabilities(
        cache_points if isinstance(cache_points, str) else "unsupported",
        supports_strict_tools=supports_strict_tools is True,
    )


def _messages(input_value: dict[str, Json]) -> list["AllMessageValues"]:
    return copy.deepcopy(cast("list[AllMessageValues]", input_value["messages"]))


def _tools(input_value: dict[str, Json]) -> list[Json]:
    return copy.deepcopy(cast("list[Json]", input_value.get("tools") or []))


def _tool_names() -> dict[str, str]:
    mappings = cast(
        "dict[str, str]",
        litellm.bedrock_tool_name_mappings.cache_dict,  # pyright: ignore[reportUnknownMemberType]
    )
    return dict(mappings)


def run(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    capabilities = _capabilities(input_value)
    model = cast(str, input_value.get("model"))
    litellm.bedrock_tool_name_mappings.cache_dict.clear()  # pyright: ignore[reportUnknownMemberType]
    with (
        patch.object(
            factory, "uuid", _FixedUuid()
        ),
        patch.object(
            converse_transformation,
            "bedrock_model_accepts_cache_points",
            capabilities.accepts_cache_points,
        ),
        patch.object(
            converse_transformation, "is_claude_4_5_on_bedrock", capabilities.extended_ttl
        ),
        patch.object(
            bedrock_common, "bedrock_model_accepts_cache_points", capabilities.accepts_cache_points
        ),
        patch.object(bedrock_common, "is_claude_4_5_on_bedrock", capabilities.extended_ttl),
        patch.object(
            bedrock_common, "bedrock_converse_supports_strict_tools", capabilities.strict_tools
        ),
    ):
        tools = _bedrock_tools_pt(_tools(input_value), model=model)
        messages = _bedrock_converse_messages_pt(
            messages=_messages(input_value),
            model=model,
            llm_provider="bedrock",
            user_continue_message=cast(
                "ChatCompletionUserMessage | None",
                copy.deepcopy(input_value.get("user_continue_message")),
            ),
            assistant_continue_message=cast(
                "str | ChatCompletionAssistantMessage | None",
                copy.deepcopy(input_value.get("assistant_continue_message")),
            ),
        )
        tool_name_mappings = _tool_names()
    return cast(
        "Json",
        {
            "messages": messages,
            "tools": tools,
            "tool_name_mappings": tool_name_mappings,
        },
    )
