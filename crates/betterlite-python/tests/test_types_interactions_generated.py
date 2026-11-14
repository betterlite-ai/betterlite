import importlib
import inspect
from enum import Enum
from types import ModuleType
from typing import Final, Protocol, TypeVar

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

PACKAGE = "betterlite.types.interactions"
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


class ReferenceModule(Protocol):
    CancelInteractionResult: type[BaseModel]
    Content: type[BaseModel]
    ContentDelta: type[BaseModel]
    CreateAgentInteractionParams: type[BaseModel]
    CreateModelInteractionParams: type[BaseModel]
    DeleteInteractionResult: type[BaseModel]
    Interaction: type[BaseModel]
    InteractionInput: TypeForm[
        str | content.Content | list[content.Content] | list[interaction.Turn]
    ]
    InteractionSseEvent: type[BaseModel]
    InteractionTool: type[BaseModel]
    InteractionToolChoiceConfig: type[BaseModel]
    InteractionEnvironment: TypeForm[str | dict[str, JsonValue]]
    # ast-grep-ignore: no-long-names
    InteractionsAPIOptionalRequestParams: TypeForm[dict[str, JsonValue]]
    InteractionsAPIResponse: type[BaseModel]
    # ast-grep-ignore: no-long-names
    InteractionsAPIStreamingResponse: type[BaseModel]
    Tool: type[BaseModel]
    ToolChoice: type[BaseModel]


THEIRS: ReferenceModule = importlib.import_module("litellm.types.interactions.generated")

CONTENT_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"type": "text", "text": "hi"},
    {"type": "image", "uri": "https://example.com/i.png", "resolution": "high"},
    {"type": "function_call", "name": "f", "arguments": {"a": 1}, "id": "c1"},
    {"type": "code_execution_result", "result": "4", "call_id": "c2"},
    {"type": "google_search_result", "result": [{"url": "u"}], "call_id": "c3"},
    {"type": "mcp_server_tool_call", "name": "t", "server_name": "s", "arguments": {}, "id": "c4"},
    {"type": "file_search_result", "result": [{"title": "t", "text": "x"}]},
    {"type": "thought", "signature": "c2ln", "summary": [{"type": "text", "text": "s"}]},
)
TOOL_PAYLOADS: Final[tuple[dict[str, JsonValue], ...]] = (
    {"type": "function", "name": "f"},
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
    {"type": "thought_summary", "content": {"type": "image", "uri": "u"}},
    {"type": "function_result", "result": {"items": ["a"]}},
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


def our_classes() -> dict[str, type]:
    found: dict[str, type] = {}
    for module in OUR_MODULES:
        found.update(defined_classes(module))
    return found


OUR_CLASSES: Final = our_classes()
THEIR_CLASSES: Final = defined_classes(
    importlib.import_module("litellm.types.interactions.generated")
)
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


def schema_view(cls: type) -> dict[str, JsonValue]:
    schema: dict[str, JsonValue] = schema_of(cls)
    properties: dict[str, JsonValue] = mapping(schema.get("properties"))
    fields: dict[str, JsonValue] = {}
    for name, property_schema in properties.items():
        fields[name] = {
            "literals": text_list(schema_literals(property_schema)),
            "default": mapping(property_schema).get("default"),
        }
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
    view: dict[str, JsonValue] = {
        "fields": schema_view(cls),
        "defs": text_list(definition_names(schema_of(cls))),
    }
    return view


def dumped_value(target: TypeForm[FormT], value: JsonValue) -> JsonValue:
    adapter: TypeAdapter[FormT] = TypeAdapter(target)
    instance: FormT = adapter.validate_python(value)
    found: JsonValue = adapter.dump_python(instance, mode="json")
    return found


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
    theirs: JsonValue = dumped_value(THEIRS.Content, payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", TOOL_PAYLOADS)
def test_names_tool_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(tools.Tool, payload)
    theirs: JsonValue = dumped_value(THEIRS.Tool, payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", EVENT_PAYLOADS)
def test_names_event_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(events.InteractionSseEvent, payload)
    theirs: JsonValue = dumped_value(THEIRS.InteractionSseEvent, payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", DELTA_PAYLOADS)
def test_names_delta_union(payload: dict[str, JsonValue]) -> None:
    ours: JsonValue = dumped_value(deltas.ContentDelta, {"delta": payload})
    theirs: JsonValue = dumped_value(THEIRS.ContentDelta, {"delta": payload})
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", INPUT_PAYLOADS)
def test_names_interaction_input(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionInput, payload)
    theirs: JsonValue = dumped_value(THEIRS.InteractionInput, payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", PARAM_PAYLOADS)
def test_names_request_alias(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionsAPIOptionalRequestParams, payload)
    theirs: JsonValue = dumped_value(THEIRS.InteractionsAPIOptionalRequestParams, payload)
    assert ours == theirs, payload


@pytest.mark.parametrize("payload", ENVIRONMENT_PAYLOADS)
def test_names_environment_alias(payload: JsonValue) -> None:
    ours: JsonValue = dumped_value(interaction.InteractionEnvironment, payload)
    theirs: JsonValue = dumped_value(THEIRS.InteractionEnvironment, payload)
    assert ours == theirs, payload


def test_names_aliases() -> None:
    assert tools.InteractionTool is tools.Tool
    assert THEIRS.InteractionTool is THEIRS.Tool
    assert tools.InteractionToolChoiceConfig is tools.ToolChoiceConfig
    assert THEIRS.InteractionToolChoiceConfig is THEIRS.ToolChoiceConfig


@pytest.mark.parametrize("name", RESPONSE_NAMES)
def test_names_litellm_responses(name: str) -> None:
    ours: JsonValue = dumped_value(OUR_CLASSES[name], EMPTY_PAYLOAD)
    theirs: JsonValue = dumped_value(THEIR_CLASSES[name], EMPTY_PAYLOAD)
    assert ours == theirs, name
    extra: JsonValue = dumped_value(OUR_CLASSES[name], {"id": "i", "extra": 1})
    assert extra == dumped_value(THEIR_CLASSES[name], {"id": "i", "extra": 1})


def test_names_tool_choice_union() -> None:
    assert dumped_value(tools.ToolChoice, "auto") == "auto"
    payload: dict[str, JsonValue] = {"allowed_tools": {"mode": "any", "tools": ["f"]}}
    ours: JsonValue = dumped_value(tools.ToolChoice, payload)
    assert ours == dumped_value(THEIRS.ToolChoice, payload)
    assert ours == payload


def test_names_computer_wire() -> None:
    payload: dict[str, JsonValue] = {
        "type": "computer_use",
        "excludedPredefinedFunctions": ["open"],
    }
    ours: JsonValue = dumped_value(tools.Tool, payload)
    assert ours == dumped_value(THEIRS.Tool, payload)
    computer: tools.ComputerUse = tools.ComputerUse.model_validate(payload)
    assert computer.excluded_predefined_functions == ["open"]
    assert computer.model_dump()["excludedPredefinedFunctions"] == ["open"]


def test_names_create_params() -> None:
    model_payload: dict[str, JsonValue] = {"model": "gemini-3", "input": "hi"}
    ours: JsonValue = dumped_value(interaction.CreateModelInteractionParams, model_payload)
    assert ours == dumped_value(THEIRS.CreateModelInteractionParams, model_payload)
    agent_payload: dict[str, JsonValue] = {
        "agent": "a",
        "input": "hi",
        "agent_config": {"type": "deep-research", "thinking_summaries": "auto"},
    }
    ours_agent: JsonValue = dumped_value(interaction.CreateAgentInteractionParams, agent_payload)
    assert ours_agent == dumped_value(THEIRS.CreateAgentInteractionParams, agent_payload)
    with pytest.raises(ValidationError, match="Field required"):
        dumped_value(interaction.CreateModelInteractionParams, {"model": "gemini-3"})
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(
            interaction.CreateAgentInteractionParams,
            {"agent": "a", "input": "hi", "agent_config": {"type": "bogus"}},
        )


def test_names_aware_datetime() -> None:
    aware: JsonValue = dumped_value(
        interaction.Interaction, {**INTERACTION_CORE, "created": AWARE_TIME}
    )
    assert aware == dumped_value(THEIRS.Interaction, {**INTERACTION_CORE, "created": AWARE_TIME})
    with pytest.raises(ValidationError, match="timezone"):
        dumped_value(interaction.Interaction, {**INTERACTION_CORE, "created": NAIVE_TIME})
    with pytest.raises(ValidationError, match="timezone"):
        dumped_value(THEIRS.Interaction, {**INTERACTION_CORE, "created": NAIVE_TIME})


def test_names_unknown_type() -> None:
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(content.Content, {"type": "bogus"})
    with pytest.raises(ValidationError, match=TAG_ERROR):
        dumped_value(THEIRS.Content, {"type": "bogus"})


def test_names_thought_summary() -> None:
    ours: JsonValue = dumped_value(OUR_CLASSES["ThoughtSummary"], THOUGHT_PAYLOAD)
    assert ours == dumped_value(THEIR_CLASSES["ThoughtSummary"], THOUGHT_PAYLOAD)
    assert ours == THOUGHT_DUMP
