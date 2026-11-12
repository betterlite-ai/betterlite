from collections.abc import Callable, Sequence
from typing import Literal, TypeAlias, TypedDict

from litellm.responses.litellm_completion_transformation.transformation import (
    TOOL_CALLS_CACHE,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RaisedTypeError(TypeError):
    pass


class RaisedAttributeError(AttributeError):
    pass


class GetterFailureError(Exception):
    pass


# Attributes are real instance attributes, so runtime Protocol checks see them; attrs mirrors them.
class Instance:
    attrs: dict[str, "Value"]
    getter: Json

    def __init__(self, attrs: dict[str, "Value"], getter: Json) -> None:
        mirror: dict[str, Value] = {}
        object.__setattr__(self, "attrs", mirror)
        object.__setattr__(self, "getter", getter)
        for name, value in attrs.items():
            self.__setattr__(name, value)

    @override
    def __setattr__(self, name: str, value: "Value") -> None:
        object.__setattr__(self, name, value)
        self.attrs[name] = value


Value: TypeAlias = (
    None
    | bool
    | int
    | float
    | str
    | bytes
    | tuple["Value", ...]
    | list["Value"]
    | dict[str, "Value"]
    | Instance
)


class MappingInstance(Instance):
    mapping: dict[str, Value]

    def __init__(self, attrs: dict[str, Value], getter: Json, mapping: dict[str, Value]) -> None:
        super().__init__(attrs, getter)
        object.__setattr__(self, "mapping", mapping)

    def get(self, key: str, default: Value) -> Value:
        if key in self.mapping:
            return self.mapping[key]
        return default


class RaisingInstance(Instance):
    raised: str

    def __init__(self, attrs: dict[str, Value], getter: Json, raised: str) -> None:
        super().__init__(attrs, getter)
        object.__setattr__(self, "raised", raised)

    def get(self, key: str, default: Value) -> Value:
        if self.raised == "type_error":
            raise RaisedTypeError(key, default)
        if self.raised == "attribute_error":
            raise RaisedAttributeError(key, default)
        raise GetterFailureError(key, default)


class RepairInput(TypedDict):
    operation: Literal["repair"]
    messages: list[Json]
    tools: list[Json]
    cache: dict[str, Json]


class RecoverInput(TypedDict):
    operation: Literal["recover"]
    assistant: Json


class CallsInput(TypedDict):
    operation: Literal["calls"]
    assistant: Json
    tool_call_id: str


class FieldInput(TypedDict):
    operation: Literal["field"]
    target: Json
    key: str


class ChunkInput(TypedDict):
    operation: Literal["chunk"]
    definition: Json
    tool_call_id: str
    index: int


class NormalizeInput(TypedDict):
    operation: Literal["normalize"]
    definition: Json
    tool_call_id: str


class ReconstructInput(TypedDict):
    operation: Literal["reconstruct"]
    tool_call_id: str
    tools: list[Json]


class AppendInput(TypedDict):
    operation: Literal["append"]
    assistant: Json
    chunk: Json


Input: TypeAlias = (
    RepairInput
    | RecoverInput
    | CallsInput
    | FieldInput
    | ChunkInput
    | NormalizeInput
    | ReconstructInput
    | AppendInput
)

Repair: TypeAlias = Callable[[Sequence[Value], Sequence[Value]], list[Value]]
Recover: TypeAlias = Callable[[Value, Value], str]
CallsList: TypeAlias = Callable[[Value], Sequence[Value]]
Exists: TypeAlias = Callable[[Sequence[Value], str], bool]
Reconstruct: TypeAlias = Callable[[str, Sequence[Value]], Value]
FieldOf: TypeAlias = Callable[[Value, str], Value]
Chunk: TypeAlias = Callable[[Value, str, int], Value]
Normalize: TypeAlias = Callable[[Value, str], Value]
Append: TypeAlias = Callable[[Value, Value], None]

repair_messages: Repair = ResponsesConfig._ensure_tool_results_have_corresponding_tool_calls  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1203
recover_call_id: Recover = ResponsesConfig._recover_tool_call_id_from_assistant  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1017
tool_calls_list: CallsList = ResponsesConfig._get_tool_calls_list  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1046
call_exists: Exists = ResponsesConfig._check_tool_call_exists  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1068
reconstruct_call: Reconstruct = ResponsesConfig._reconstruct_tool_call_from_tools  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1081
field_of: FieldOf = ResponsesConfig._get_mapping_or_attr_value  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1100
create_chunk: Chunk = ResponsesConfig._create_tool_call_chunk  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1120
normalize_definition: Normalize = ResponsesConfig._normalize_tool_use_definition  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1146
append_call: Append = ResponsesConfig._add_tool_call_to_assistant  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L1187


def instance_from_wire(attrs: dict[str, Json], getter: Json) -> Instance:
    fields = {name: from_wire(item) for name, item in attrs.items()}
    match getter:
        case {"mapping": dict(mapping)}:
            lookup = {name: from_wire(item) for name, item in mapping.items()}
            return MappingInstance(fields, getter, lookup)
        case {"raises": str(raised)}:
            return RaisingInstance(fields, getter, raised)
        case other:
            return Instance(fields, other)


def from_wire(wire: Json) -> Value:
    match wire:
        case None | bool() | int() | float() | str():
            return wire
        case list():
            return [from_wire(item) for item in wire]
        case {"__tuple__": list(items)}:
            return tuple(from_wire(item) for item in items)
        case {"__bytes__": str(text)}:
            return text.encode()
        case {"__object__": {"attrs": dict(attrs), "get": (dict() | str()) as getter}}:
            return instance_from_wire(attrs, getter)
        case dict():
            return {name: from_wire(item) for name, item in wire.items()}


def to_wire(value: Value) -> Json:
    match value:
        case None | bool() | int() | float() | str():
            return value
        case bytes():
            return {"__bytes__": value.decode()}
        case tuple():
            return {"__tuple__": [to_wire(item) for item in value]}
        case list():
            return [to_wire(item) for item in value]
        case dict():
            return {name: to_wire(item) for name, item in value.items()}
        case Instance():
            attrs = {name: to_wire(item) for name, item in value.attrs.items()}
            return {"__object__": {"attrs": attrs, "get": value.getter}}


def repair(input_value: RepairInput) -> Json:
    messages = [from_wire(message) for message in input_value["messages"]]
    tools = [from_wire(tool) for tool in input_value["tools"]]
    for call_id, definition in input_value["cache"].items():
        TOOL_CALLS_CACHE.set_cache(key=call_id, value=from_wire(definition))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L158
    repaired = repair_messages(messages, tools)
    return {
        "repaired": [to_wire(message) for message in repaired],
        "input": [to_wire(message) for message in messages],
    }


def calls(input_value: CallsInput) -> Json:
    tool_calls = tool_calls_list(from_wire(input_value["assistant"]))
    return {
        "calls": [to_wire(item) for item in tool_calls],
        "exists": call_exists(tool_calls, input_value["tool_call_id"]),
    }


def run_operation(input_value: Input) -> Json:
    result: Json
    if input_value["operation"] == "repair":
        result = repair(input_value)
    elif input_value["operation"] == "recover":
        assistant = from_wire(input_value["assistant"])
        result = {"id": recover_call_id(assistant, assistant)}
    elif input_value["operation"] == "calls":
        result = calls(input_value)
    elif input_value["operation"] == "field":
        target = from_wire(input_value["target"])
        result = {"value": to_wire(field_of(target, input_value["key"]))}
    elif input_value["operation"] == "chunk":
        definition = from_wire(input_value["definition"])
        chunk = create_chunk(definition, input_value["tool_call_id"], input_value["index"])
        result = {"chunk": to_wire(chunk)}
    elif input_value["operation"] == "normalize":
        definition = from_wire(input_value["definition"])
        normalized = normalize_definition(definition, input_value["tool_call_id"])
        result = {"definition": to_wire(normalized)}
    elif input_value["operation"] == "reconstruct":
        tools = [from_wire(tool) for tool in input_value["tools"]]
        result = {"definition": to_wire(reconstruct_call(input_value["tool_call_id"], tools))}
    else:
        assistant = from_wire(input_value["assistant"])
        append_call(assistant, from_wire(input_value["chunk"]))
        result = {"assistant": to_wire(assistant)}
    return result


def run(input_value: Input) -> Json:
    try:
        return run_operation(input_value)
    except (RaisedTypeError, RaisedAttributeError, GetterFailureError):
        return {"error": "getter_failed"}
    except AttributeError:
        return {"error": "no_get_method"}
