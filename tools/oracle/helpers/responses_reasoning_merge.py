from dataclasses import dataclass, field
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

_KNOWN_FIELDS = frozenset({"role", "content", "reasoning_content", "thinking_blocks", "tool_calls"})


@dataclass
class AttributeChatRecord:
    role: Json = None
    role_present: bool = False
    content: Json = None
    reasoning_content: Json = None
    thinking_blocks: Json = None
    tool_calls: Json = None
    extra: dict[str, Json] = field(default_factory=dict)

    def __getattr__(self, name: str) -> Json:
        # Extra attributes remain readable because real message classes use extra="allow".
        try:
            return self.extra[name]
        except KeyError as error:
            raise AttributeError(name) from error


def _as_dict(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError("expected a JSON object")
    return value


def _as_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("expected a JSON array")
    return value


def _to_attribute_record(message: Json) -> AttributeChatRecord:
    fields = _as_dict(message)
    role_present = "role" in fields
    role = fields.get("role")
    extra = {key: value for key, value in fields.items() if key not in _KNOWN_FIELDS}
    return AttributeChatRecord(
        role=role,
        role_present=role_present,
        content=fields.get("content"),
        reasoning_content=fields.get("reasoning_content"),
        thinking_blocks=fields.get("thinking_blocks"),
        tool_calls=fields.get("tool_calls"),
        extra=extra,
    )


def _from_attribute_record(record: AttributeChatRecord) -> dict[str, Json]:
    result: dict[str, Json] = {"content": record.content}
    if record.role_present:
        result["role"] = record.role
    if record.reasoning_content is not None:
        result["reasoning_content"] = record.reasoning_content
    if record.thinking_blocks is not None:
        result["thinking_blocks"] = record.thinking_blocks
    if record.tool_calls is not None:
        result["tool_calls"] = record.tool_calls
    result.update(record.extra)
    return result


def _merge_reasoning(messages: "list[ReasoningMessage]") -> "list[ReasoningMessage]":
    config = LiteLLMCompletionResponsesConfig
    return config._merge_reasoning_only_assistant_messages(messages)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def _record_kinds(input_value: dict[str, Json], count: int) -> list[str]:
    raw = input_value.get("record_kind", "dict")
    kinds = [str(item) for item in raw] if isinstance(raw, list) else [str(raw)] * count
    if len(kinds) != count:
        raise ValueError("record_kind length must match messages length")
    return kinds


def _merge_mixed_records(messages: list[Json], kinds: list[str]) -> Json:
    built: list[object] = [
        _to_attribute_record(message) if kind == "attribute" else _as_dict(message)
        for message, kind in zip(messages, kinds, strict=True)
    ]
    # cast-ok: each entry is read only through isinstance(dict)/getattr, matching litellm's dispatch
    merged = _merge_reasoning(cast("list[ReasoningMessage]", built))
    result: list[Json] = []
    for item in merged:
        if isinstance(item, AttributeChatRecord):
            result.append(_from_attribute_record(item))
        else:
            # cast-ok: every other merged entry is a plain dict at runtime (passthrough or rebuilt)
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
        kinds = _record_kinds(input_value, len(messages))
        return _merge_mixed_records(messages, kinds)
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
