import importlib
import inspect
from enum import Enum
from types import ModuleType
from typing import Final, TypeVar

import pytest
from pydantic import BaseModel, JsonValue, TypeAdapter, ValidationError
from typing_extensions import TypeForm

from betterlite.types.interactions import (
    content,
    deltas,
    enums,
    events,
    interaction,
    litellm_responses,
    options,
    tools,
)

FormT = TypeVar("FormT")
OUR_MODULES: Final = (
    content,
    deltas,
    enums,
    events,
    interaction,
    litellm_responses,
    options,
    tools,
)
OPTION_TEXT: Final = "gemini-3-pro"
INTERACTION_CORE: Final[dict[str, JsonValue]] = {"id": "i", "status": "completed"}
EMPTY_PAYLOAD: Final[dict[str, JsonValue]] = {}
AWARE_TIME: Final = "2026-01-01T00:00:00Z"
NAIVE_TIME: Final = "2026-01-01T00:00:00"
TAG_ERROR: Final = "does not match any of the expected tags"
JSON_SCHEMA_REF: Final = "#/$defs/JsonValue"


CONTENT_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"type": "text", "text": "hi"},
    {"type": "image", "uri": "https://example.com/i.png", "resolution": "high"},
    {"type": "audio", "uri": "u", "mime_type": "audio/wav"},
    {"type": "document", "uri": "u", "mime_type": "text/plain"},
    {"type": "video", "uri": "u", "mime_type": "video/mp4", "resolution": "low"},
    {"type": "thought", "signature": "c2ln", "summary": [{"type": "text", "text": "s"}]},
    {"type": "function_call", "name": "f", "arguments": {"a": 1}, "id": "c1"},
    {"type": "function_result", "name": "f", "result": {"items": ["a"]}, "call_id": "c5"},
    {"type": "code_execution_call", "arguments": {"language": "python", "code": "1"}, "id": "c6"},
    {"type": "code_execution_result", "result": "4", "call_id": "c2"},
    {"type": "url_context_call", "arguments": {"urls": ["https://u"]}, "id": "c7"},
    {"type": "url_context_result", "result": [{"url": "u", "status": "success"}], "call_id": "c8"},
    {"type": "google_search_call", "arguments": {"queries": ["q"]}, "id": "c9"},
    {"type": "google_search_result", "result": [{"url": "u"}], "call_id": "c3"},
    {"type": "mcp_server_tool_call", "name": "t", "server_name": "s", "arguments": {}, "id": "c4"},
    {
        "type": "mcp_server_tool_result",
        "name": "t",
        "server_name": "s",
        "result": "x",
        "call_id": "c10",
    },
    {"type": "file_search_result", "result": [{"title": "t", "text": "x"}]},
)
TOOL_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"type": "function", "name": "f"},
    {"type": "function", "parameters": {"type": "object", "properties": {}}},
    {"type": "function", "parameters": True},
    {"type": "function", "parameters": ["a"]},
    {"type": "function", "parameters": "s"},
    {"type": "google_search"},
    {"type": "code_execution"},
    {"type": "url_context"},
    {"type": "mcp_server", "name": "m", "url": "u", "allowed_tools": [{"mode": "auto"}]},
    {"type": "file_search", "top_k": 2},
    {"type": "computer_use", "environment": "browser", "excludedPredefinedFunctions": ["open"]},
)
EVENT_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"event_type": "step.start", "index": 0, "step": {"type": "text"}},
    {"event_type": "step.delta", "index": 0, "delta": {"text": "x"}},
    {"event_type": "step.stop", "index": 0, "status": "done"},
    {"event_type": "interaction.created", "interaction": {"id": "i"}},
    {"event_type": "interaction.in_progress", "interaction_id": "i"},
    {"event_type": "interaction.completed", "interaction": {"id": "i"}},
    {"event_type": "interaction.requires_action", "interaction_id": "i"},
    {"event_type": "interaction.start", "interaction": {"id": "i", "status": "queued"}},
    {"event_type": "interaction.status_update", "status": "in_progress"},
    {"event_type": "content.start", "index": 1, "content": {"type": "text", "text": "t"}},
    {"event_type": "content.delta", "delta": {"type": "text", "text": "d"}},
    {"event_type": "content.stop"},
    {"event_type": "error", "error": {"code": "c", "message": "m"}},
)
DELTA_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"type": "text", "text": "x"},
    {"type": "image", "uri": "u", "mime_type": "image/png", "resolution": "high"},
    {"type": "audio", "uri": "u", "mime_type": "audio/wav"},
    {"type": "document", "uri": "u", "mime_type": "text/plain"},
    {"type": "video", "uri": "u", "mime_type": "video/mp4", "resolution": "medium"},
    {"type": "thought_summary", "content": {"type": "image", "uri": "u"}},
    {"type": "thought_signature", "signature": "c2ln"},
    {"type": "function_call", "name": "f", "arguments": {"a": 1}, "id": "d1"},
    {"type": "function_result", "result": {"items": ["a"]}},
    {"type": "code_execution_call", "arguments": {"language": "python", "code": "1"}, "id": "d2"},
    {"type": "code_execution_result", "result": "4", "call_id": "d3"},
    {"type": "url_context_call", "arguments": {"urls": ["u"]}, "id": "d4"},
    {"type": "url_context_result", "result": [{"url": "u", "status": "error"}], "call_id": "d5"},
    {"type": "google_search_call", "arguments": {"queries": ["q"]}, "id": "d6"},
    {"type": "google_search_result", "result": [{"url": "u", "title": "t"}], "call_id": "d7"},
    {
        "type": "mcp_server_tool_call",
        "name": "t",
        "server_name": "s",
        "arguments": {"a": 1},
        "id": "d8",
    },
    {
        "type": "mcp_server_tool_result",
        "name": "t",
        "server_name": "s",
        "result": "done",
        "call_id": "d9",
    },
    {"type": "file_search_result", "result": []},
)
INPUT_PAYLOADS: Final[tuple[JsonValue, ...]] = (
    "hello",
    {"type": "text", "text": "hi"},
    [{"type": "text", "text": "a"}, {"type": "image", "uri": "u"}],
    [{"role": "user", "content": "hi"}],
    [{"role": "model", "content": [{"type": "text", "text": "x"}]}],
)
ENVIRONMENT_PAYLOADS: Final[tuple[JsonValue, ...]] = ("browser", {"a": 1})
PARAM_PAYLOADS: Final[tuple[JsonValue, ...]] = ({}, {"model": "m"}, {"x": [1, None]})
OPTION_NAMES: Final = (
    "ModelOption",
    "AgentOption",
    "ImageMimeTypeOption",
    "AudioMimeTypeOption",
    "VideoMimeTypeOption",
)
RESPONSE_NAMES: Final = (
    "InteractionsAPIResponse",
    "InteractionsAPIStreamingResponse",
    "DeleteInteractionResult",
    "CancelInteractionResult",
)
MODEL_CORE: Final[dict[str, JsonValue]] = {"model": "gemini-3", "input": "hi"}
AGENT_CORE: Final[dict[str, JsonValue]] = {"agent": "a", "input": "hi"}
DATETIME_CASES: Final[tuple[tuple[str, str, dict[str, JsonValue]], ...]] = (
    ("Interaction", "created", INTERACTION_CORE),
    ("Interaction", "updated", INTERACTION_CORE),
    ("CreateModelInteractionParams", "created", MODEL_CORE),
    ("CreateModelInteractionParams", "updated", MODEL_CORE),
    ("CreateAgentInteractionParams", "created", AGENT_CORE),
    ("CreateAgentInteractionParams", "updated", AGENT_CORE),
)
THOUGHT_PAYLOAD: Final[list[JsonValue]] = [
    {"type": "text", "text": "a"},
    {"type": "image", "uri": "u"},
]
THOUGHT_DUMP: Final[list[JsonValue]] = [
    {"text": "a", "type": "text", "annotations": None},
    {"data": None, "uri": "u", "mime_type": None, "type": "image", "resolution": None},
]


def defined_classes(module: ModuleType) -> dict[str, type]:
    members: list[tuple[str, type]] = inspect.getmembers(module, inspect.isclass)
    found: dict[str, type] = {
        name: member for name, member in members if member.__module__ == module.__name__
    }
    return found


def defined_forms(module: ModuleType) -> dict[str, TypeForm[JsonValue]]:
    members: list[tuple[str, TypeForm[JsonValue]]] = inspect.getmembers(module)
    found: dict[str, TypeForm[JsonValue]] = dict(members)
    return found


def our_classes() -> dict[str, type]:
    found: dict[str, type] = {}
    for module in OUR_MODULES:
        found.update(defined_classes(module))
    return found


THEIRS: Final[ModuleType] = importlib.import_module("litellm.types.interactions.generated")
OUR_CLASSES: Final = our_classes()
THEIR_CLASSES: Final = defined_classes(THEIRS)
THEIR_FORMS: Final = defined_forms(THEIRS)
CLASS_NAMES: Final = tuple(sorted(THEIR_CLASSES))


def text_list(values: list[str]) -> list[JsonValue]:
    found: list[JsonValue] = []
    found.extend(values)
    return found


def schema_of(target: TypeForm[FormT]) -> dict[str, JsonValue]:
    found: dict[str, JsonValue] = TypeAdapter(target).json_schema()
    return found


def mapping(value: JsonValue) -> dict[str, JsonValue]:
    if value is None:
        return {}
    match value:
        case dict():
            return value
        case list() | str() | bool() | int() | float():
            raise TypeError("a JSON schema object was expected")


def strings(value: JsonValue) -> list[str]:
    if value is None:
        return []
    match value:
        case list():
            found: list[str] = []
            for entry in value:
                match entry:
                    case str():
                        found.append(entry)
                    case dict() | list() | bool() | int() | float() | None:
                        raise TypeError("a JSON schema string list was expected")
            return found
        case dict() | str() | bool() | int() | float():
            raise TypeError("a JSON schema list was expected")


def schema_literals(value: JsonValue) -> list[str]:
    found: list[str] = []
    stack: list[JsonValue] = [value]
    while stack:
        current: JsonValue = stack.pop()
        match current:
            case dict():
                for key, item in current.items():
                    if key in {"const", "enum"}:
                        found.append(repr(item))
                    else:
                        stack.append(item)
            case list():
                stack.extend(current)
            case str() | bool() | int() | float() | None:
                continue
    return sorted(found)


def definition_names(schema: JsonValue) -> list[str]:
    definitions: JsonValue = mapping(schema).get("$defs")
    if definitions is None:
        return []
    names: dict[str, JsonValue] = mapping(definitions)
    return sorted(name for name in names if name != "JsonValue")


def schema_shape(value: JsonValue) -> JsonValue:
    match value:
        case dict():
            return dict_shape(value)
        case list():
            return [schema_shape(item) for item in value]
        case str() | bool() | int() | float() | None:
            return value


def dict_shape(value: dict[str, JsonValue]) -> JsonValue:
    found: dict[str, JsonValue] = {}
    for key, item in value.items():
        if key in {"title", "description"}:
            continue
        if key == "$ref" and item == JSON_SCHEMA_REF:
            found["anyOf"] = [{}, {"type": "null"}]
            continue
        if key == "additionalProperties" and item == {"$ref": JSON_SCHEMA_REF}:
            found[key] = True
            continue
        found[key] = schema_shape(item)
    return found


def schema_view(schema: dict[str, JsonValue]) -> dict[str, JsonValue]:
    properties: dict[str, JsonValue] = mapping(schema.get("properties"))
    fields: dict[str, JsonValue] = {}
    for name, property_schema in properties.items():
        fields[name] = schema_shape(property_schema)
    view: dict[str, JsonValue] = {
        "required": text_list(strings(schema.get("required"))),
        "keys": text_list(sorted(properties)),
        "fields": fields,
    }
    return view


def enum_view(cls: type) -> dict[str, JsonValue]:
    members: list[str] = [name for name in dir(cls) if not name.startswith("_")]
    view: dict[str, JsonValue] = {
        "values": text_list(schema_literals(schema_of(cls))),
        "members": text_list(members),
    }
    return view


def class_view(cls: type) -> dict[str, JsonValue]:
    if Enum in cls.__mro__:
        return enum_view(cls)
    schema: dict[str, JsonValue] = schema_of(cls)
    root: dict[str, JsonValue] = {
        key: value for key, value in schema.items() if key not in {"properties", "$defs"}
    }
    view: dict[str, JsonValue] = {
        "root": schema_shape(root),
        "fields": schema_view(schema),
        "defs": text_list(definition_names(schema)),
    }
    return view


def dumped_value(target: TypeForm[FormT], value: JsonValue) -> JsonValue:
    adapter: TypeAdapter[FormT] = TypeAdapter(target)
    instance: FormT = adapter.validate_python(value)
    found: JsonValue = adapter.dump_python(instance, mode="json")
    return found


def hidden_params(instance: BaseModel) -> dict[str, JsonValue]:
    private: dict[str, JsonValue] | None = instance.__pydantic_private__
    assert private is not None
    hidden: JsonValue = private["_hidden_params"]
    return mapping(hidden)


def test_names_class_set() -> None:
    missing: list[str] = sorted(set(THEIR_CLASSES) - set(OUR_CLASSES))
    extra: list[str] = sorted(set(OUR_CLASSES) - set(THEIR_CLASSES))
    assert missing == []
    assert extra == []


@pytest.mark.parametrize("name", CLASS_NAMES)
def test_names_class_views(name: str) -> None:
    assert name in OUR_CLASSES, f"{name} is missing"
    ours: type = OUR_CLASSES[name]
    theirs: type = THEIR_CLASSES[name]
    assert class_view(ours) == class_view(theirs), name


@pytest.mark.parametrize("name", OPTION_NAMES)
def test_names_option_roots(name: str) -> None:
    assert dumped_value(OUR_CLASSES[name], OPTION_TEXT) == OPTION_TEXT
    assert dumped_value(THEIR_CLASSES[name], OPTION_TEXT) == OPTION_TEXT


@pytest.mark.parametrize("payload", CONTENT_PAYLOADS)
def test_names_content_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(content.Content, payload)
    theirs: JsonValue = dumped_value(THEIR_CLASSES["Content"], payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", TOOL_PAYLOADS)
def test_names_tool_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(tools.Tool, payload)
    theirs: JsonValue = dumped_value(THEIR_CLASSES["Tool"], payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", EVENT_PAYLOADS)
def test_names_event_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(events.InteractionSseEvent, payload)
    theirs: JsonValue = dumped_value(THEIR_CLASSES["InteractionSseEvent"], payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", DELTA_PAYLOADS)
def test_names_delta_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(deltas.ContentDelta, {"delta": payload})
    theirs: JsonValue = dumped_value(THEIR_CLASSES["ContentDelta"], {"delta": payload})
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", INPUT_PAYLOADS)
def test_names_interaction_input(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionInput, payload)
    theirs: JsonValue = dumped_value(THEIR_FORMS["InteractionInput"], payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", PARAM_PAYLOADS)
def test_names_request_alias(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionsAPIOptionalRequestParams, payload)
    theirs: JsonValue = dumped_value(THEIR_FORMS["InteractionsAPIOptionalRequestParams"], payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", ENVIRONMENT_PAYLOADS)
def test_names_environment_alias(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionEnvironment, payload)
    theirs: JsonValue = dumped_value(THEIR_FORMS["InteractionEnvironment"], payload)
    assert ours == theirs, payload


def test_names_aliases() -> None:
    assert tools.InteractionTool is tools.Tool
    assert THEIR_CLASSES["InteractionTool"] is THEIR_CLASSES["Tool"]
    assert tools.InteractionToolChoiceConfig is tools.ToolChoiceConfig
    assert THEIR_CLASSES["InteractionToolChoiceConfig"] is THEIR_CLASSES["ToolChoiceConfig"]


@pytest.mark.parametrize("name", RESPONSE_NAMES)
def test_names_litellm_responses(name: str) -> None:
    ours: JsonValue = dumped_value(OUR_CLASSES[name], EMPTY_PAYLOAD)
    theirs: JsonValue = dumped_value(THEIR_CLASSES[name], EMPTY_PAYLOAD)
    assert ours == theirs, name
    extra: JsonValue = dumped_value(OUR_CLASSES[name], {"id": "i", "extra": 1})
    assert extra == dumped_value(THEIR_CLASSES[name], {"id": "i", "extra": 1})
    payload: dict[str, JsonValue] = {
        "id": "i",
        "outputs": [{"type": "text", "text": "x"}],
        "usage": {"total_tokens": 1},
    }
    assert dumped_value(OUR_CLASSES[name], payload) == dumped_value(THEIR_CLASSES[name], payload)
    ours_cls: type[BaseModel] = OUR_CLASSES[name]
    theirs_cls: type[BaseModel] = THEIR_CLASSES[name]
    assert ours_cls.__private_attributes__.keys() == theirs_cls.__private_attributes__.keys()
    assert ours_cls.__private_attributes__["_hidden_params"].default_factory is dict
    our_adapter: TypeAdapter[BaseModel] = TypeAdapter(ours_cls)
    their_adapter: TypeAdapter[BaseModel] = TypeAdapter(theirs_cls)
    our_instance: BaseModel = our_adapter.validate_python(EMPTY_PAYLOAD)
    their_instance: BaseModel = their_adapter.validate_python(EMPTY_PAYLOAD)
    assert hidden_params(our_instance) == {}
    assert hidden_params(their_instance) == {}
    second: BaseModel = our_adapter.validate_python(EMPTY_PAYLOAD)
    assert hidden_params(second) is not hidden_params(our_instance)


def test_names_tool_choice_union() -> None:
    assert dumped_value(tools.ToolChoice, "auto") == "auto"
    payload: dict[str, JsonValue] = {"allowed_tools": {"mode": "any", "tools": ["f"]}}
    ours: JsonValue = dumped_value(tools.ToolChoice, payload)
    assert ours == dumped_value(THEIR_CLASSES["ToolChoice"], payload)
    assert ours == payload


def test_names_computer_wire() -> None:
    payload: dict[str, JsonValue] = {
        "type": "computer_use",
        "excludedPredefinedFunctions": ["open"],
    }
    ours: JsonValue = dumped_value(tools.Tool, payload)
    assert ours == dumped_value(THEIR_CLASSES["Tool"], payload)
    computer: tools.ComputerUse = tools.ComputerUse.model_validate(payload)
    assert computer.excluded_predefined_functions == ["open"]
    assert computer.model_dump()["excludedPredefinedFunctions"] == ["open"]


def test_names_create_params() -> None:
    model_payload: dict[str, JsonValue] = {
        "model": "gemini-3",
        "input": "hi",
        "response_format": {"type": "object"},
    }
    ours: JsonValue = dumped_value(interaction.CreateModelInteractionParams, model_payload)
    assert ours == dumped_value(THEIR_CLASSES["CreateModelInteractionParams"], model_payload)
    agent_payload: dict[str, JsonValue] = {
        "agent": "a",
        "input": "hi",
        "agent_config": {"type": "deep-research", "thinking_summaries": "auto"},
    }
    ours_agent: JsonValue = dumped_value(interaction.CreateAgentInteractionParams, agent_payload)
    assert ours_agent == dumped_value(THEIR_CLASSES["CreateAgentInteractionParams"], agent_payload)
    with pytest.raises(ValidationError, match="Field required"):
        dumped_value(interaction.CreateModelInteractionParams, {"model": "gemini-3"})
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(
            interaction.CreateAgentInteractionParams,
            {"agent": "a", "input": "hi", "agent_config": {"type": "bogus"}},
        )


@pytest.mark.parametrize(("name", "field", "base"), DATETIME_CASES)
def test_names_aware_datetime(name: str, field: str, base: dict[str, JsonValue]) -> None:
    aware_payload: dict[str, JsonValue] = {**base, field: AWARE_TIME}
    ours: JsonValue = dumped_value(OUR_CLASSES[name], aware_payload)
    assert ours == dumped_value(THEIR_CLASSES[name], aware_payload), name
    naive_payload: dict[str, JsonValue] = {**base, field: NAIVE_TIME}
    with pytest.raises(ValidationError, match="timezone"):
        dumped_value(OUR_CLASSES[name], naive_payload)
    with pytest.raises(ValidationError, match="timezone"):
        dumped_value(THEIR_CLASSES[name], naive_payload)


def test_names_unknown_type() -> None:
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(content.Content, {"type": "bogus"})
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(THEIR_CLASSES["Content"], {"type": "bogus"})


def test_names_event_tag() -> None:
    payload: dict[str, JsonValue] = {"index": 0}
    with pytest.raises(ValidationError, match="event_type"):
        dumped_value(events.InteractionSseEvent, payload)
    with pytest.raises(ValidationError, match="event_type"):
        dumped_value(THEIR_CLASSES["InteractionSseEvent"], payload)


def test_names_thought_summary() -> None:
    ours: JsonValue = dumped_value(OUR_CLASSES["ThoughtSummary"], THOUGHT_PAYLOAD)
    assert ours == dumped_value(THEIR_CLASSES["ThoughtSummary"], THOUGHT_PAYLOAD)
    assert ours == THOUGHT_DUMP
