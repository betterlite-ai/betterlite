import json
from collections.abc import Callable
from types import SimpleNamespace
from typing import Protocol, TypeAlias, cast

import litellm
from litellm.integrations.arize import _utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class AttributeSpan(Protocol):
    def set_attribute(self, key: str, value: str | float) -> None: ...


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict[str, Json] = {}

    def set_attribute(self, key: str, value: str | float) -> None:
        self.attributes[key] = value


set_messages = cast(
    Callable[[AttributeSpan, dict[str, Json]], None],
    cast(object, _utils.ArizeOTELAttributes.set_messages),
)
set_response_attributes = cast(
    Callable[[AttributeSpan, Json], None],
    cast(object, _utils._set_response_attributes),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)
coerce_text = cast(
    Callable[[Json], str | None],
    cast(object, _utils._coerce_text),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)
summarize_tool_calls = cast(
    Callable[[list[Json]], str],
    cast(object, _utils._summarize_tool_calls_for_output),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("function"):
        case "set_messages":
            return {"result": set_messages_record(input_value)}
        case "set_response":
            return {"result": set_response_record(input_value)}
        case "coerce_text":
            return {"result": coerce_text(input_value.get("value"))}
        case "summarize_tool_calls":
            return {"result": summarize_record(input_value)}
        case other:
            raise ValueError(f"unknown b3860 function {other} ({litellm.__name__})")


def set_messages_record(input_value: dict[str, Json]) -> Json:
    messages = array_value(input_value.get("messages"), "messages")
    span = RecordingSpan()
    set_messages(span, {"messages": messages})
    record = object_value(normalize_value(span.attributes, ""), "record")
    drop_array_contents(record, messages)
    return record


def set_response_record(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    set_response_attributes(span, response_object(input_value.get("response")))
    return normalize_value(span.attributes, "")


def summarize_record(input_value: dict[str, Json]) -> Json:
    tool_calls = array_value(input_value.get("tool_calls"), "tool_calls")
    summary = summarize_tool_calls(tool_calls)
    return normalize_value(cast(Json, json.loads(summary)), "")


def response_object(response: Json) -> Json:
    if not isinstance(response, dict):
        return response
    output = response.get("output")
    if not isinstance(output, list):
        return response
    converted = dict(response)
    converted["output"] = [namespace_value(item) for item in output]
    return converted


def namespace_value(value: Json) -> Json:
    if isinstance(value, dict):
        fields = {key: namespace_value(item) for key, item in value.items()}
        return cast(Json, cast(object, SimpleNamespace(**fields)))
    if isinstance(value, list):
        return [namespace_value(item) for item in value]
    return value


def drop_array_contents(record: dict[str, Json], messages: list[Json]) -> None:
    last = messages[-1] if messages else None
    if isinstance(last, dict) and isinstance(last.get("content"), list):
        record.pop("input.value", None)
    for index, message in enumerate(messages):
        if isinstance(message, dict) and isinstance(message.get("content"), list):
            record.pop(f"llm.input_messages.{index}.message.content", None)


def normalize_value(value: Json, path: str) -> Json:
    if isinstance(value, dict):
        normalized: dict[str, Json] = {}
        for key, item in value.items():
            normalized[key] = normalize_value(item, child_path(path, key))
        return normalized
    if isinstance(value, list):
        return [normalize_value(item, f"{path}.{index}") for index, item in enumerate(value)]
    if isinstance(value, str) and (path == "output.value" or path.endswith(".function.arguments")):
        parsed_ok, parsed = parse_json(value)
        return normalize_value(parsed, path) if parsed_ok else value
    return value


def child_path(path: str, key: str) -> str:
    return key if not path else f"{path}.{key}"


def parse_json(text: str) -> tuple[bool, Json]:
    try:
        return True, cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return False, text


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be an array")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value
