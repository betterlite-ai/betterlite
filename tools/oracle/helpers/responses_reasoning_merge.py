from dataclasses import dataclass
from typing import TypeAlias, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig,
)
from litellm.types.llms.openai import (
    AllMessageValues,
    ChatCompletionAssistantMessage,
    ChatCompletionResponseMessage,
    GenericChatCompletionMessage,
)
from litellm.types.utils import ChatCompletionMessageToolCall

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

ReasoningMessage: TypeAlias = (
    AllMessageValues
    | GenericChatCompletionMessage
    | ChatCompletionMessageToolCall
    | ChatCompletionResponseMessage
)
TrailingMessage: TypeAlias = (
    AllMessageValues | GenericChatCompletionMessage | ChatCompletionResponseMessage
)


@dataclass
class AttributeChatRecord:
    role: str
    content: str | None = None
    reasoning_content: str | None = None
    thinking_blocks: list[Json] | None = None
    tool_calls: list[Json] | None = None


def _as_dict(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError("expected a JSON object")
    return value


def _as_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("expected a JSON array")
    return value


def _as_str_or_none(value: Json, field: str) -> str | None:
    if value is not None and not isinstance(value, str):
        raise ValueError(f"{field} must be a string or null")
    return value


def _to_attribute_record(message: Json) -> AttributeChatRecord:
    fields = _as_dict(message)
    role = fields.get("role")
    if not isinstance(role, str):
        raise ValueError("role must be a string")
    thinking_blocks = fields.get("thinking_blocks")
    tool_calls = fields.get("tool_calls")
    return AttributeChatRecord(
        role=role,
        content=_as_str_or_none(fields.get("content"), "content"),
        reasoning_content=_as_str_or_none(fields.get("reasoning_content"), "reasoning_content"),
        thinking_blocks=_as_list(thinking_blocks) if thinking_blocks is not None else None,
        tool_calls=_as_list(tool_calls) if tool_calls is not None else None,
    )


def _from_attribute_record(record: AttributeChatRecord) -> dict[str, Json]:
    result: dict[str, Json] = {"role": record.role, "content": record.content}
    if record.reasoning_content is not None:
        result["reasoning_content"] = record.reasoning_content
    if record.thinking_blocks is not None:
        result["thinking_blocks"] = record.thinking_blocks
    if record.tool_calls is not None:
        result["tool_calls"] = record.tool_calls
    return result


def _merge_reasoning(messages: "list[ReasoningMessage]") -> "list[ReasoningMessage]":
    config = LiteLLMCompletionResponsesConfig
    return config._merge_reasoning_only_assistant_messages(messages)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _merge_dict_records(messages: list[Json]) -> Json:
    # cast-ok: each entry is read only through isinstance(dict)/dict.get, never constructed
    merged = _merge_reasoning(cast("list[ReasoningMessage]", messages))
    # cast-ok: every merged entry is itself a plain dict at runtime
    return cast("list[Json]", merged)


def _merge_attribute_records(messages: list[Json]) -> Json:
    records = [_to_attribute_record(message) for message in messages]
    # cast-ok: non-dict records are read only through getattr, matching litellm's dynamic dispatch
    merged = _merge_reasoning(cast("list[ReasoningMessage]", records))
    result: list[Json] = []
    for item in merged:
        if isinstance(item, AttributeChatRecord):
            result.append(_from_attribute_record(item))
        else:
            # cast-ok: every other merged entry is a plain dict at runtime (a standalone rebuild)
            result.append(cast("dict[str, Json]", item))
    return result


def _merge_trailing(
    messages: "list[TrailingMessage]",
    new_messages: "list[TrailingMessage]",
    *,
    hosted_search: bool,
) -> "ChatCompletionAssistantMessage | None":
    config = LiteLLMCompletionResponsesConfig
    return config._merged_trailing_assistant_message(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        messages=messages,
        chat_completion_messages=new_messages,
        hosted_search=hosted_search,
    )


def run(input_value: dict[str, Json]) -> Json:
    mode = input_value["mode"]
    if mode == "reasoning":
        messages = _as_list(input_value["messages"])
        record_kind = input_value.get("record_kind", "dict")
        if record_kind == "attribute":
            return _merge_attribute_records(messages)
        return _merge_dict_records(messages)
    if mode == "trailing":
        messages = _as_list(input_value["messages"])
        new_messages = _as_list(input_value.get("new_messages", []))
        hosted_search = input_value.get("hosted_search", False)
        if not isinstance(hosted_search, bool):
            raise ValueError("hosted_search must be a boolean")
        # cast-ok: read only through isinstance(dict), never constructed
        merged = _merge_trailing(
            cast("list[TrailingMessage]", messages),
            cast("list[TrailingMessage]", new_messages),
            hosted_search=hosted_search,
        )
        # a bare null here drops the wire format's "value" key on the Option<Value> round trip
        # cast-ok: the assistant TypedDict is a plain dict at runtime
        return {"merged": cast("Json", merged)}
    raise ValueError("unknown merge mode")
