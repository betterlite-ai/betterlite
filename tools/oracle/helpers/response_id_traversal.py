import logging
from collections.abc import Mapping
from functools import partial
from types import TracebackType
from typing import Literal, NotRequired, Self, TypeAlias, TypedDict, TypeVar

from litellm.responses.utils import ResponsesAPIRequestUtils
from litellm.types.llms.openai import ResponsesAPIResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class ObjectWire(TypedDict):
    fields: dict[str, JsonValue]
    readonly: list[str]


class FixtureObject:
    fields: dict[str, "Value"]
    readonly: frozenset[str]

    def __init__(self, fields: dict[str, "Value"], readonly: list[str]) -> None:
        frozen: frozenset[str] = frozenset(readonly)
        object.__setattr__(self, "readonly", frozen)
        mirror: dict[str, Value] = {}
        object.__setattr__(self, "fields", mirror)
        for name, item in fields.items():
            object.__setattr__(self, name, item)
            mirror[name] = item

    @override
    def __setattr__(self, name: str, value: "Value") -> None:
        if name in self.readonly:
            raise AttributeError(name)
        object.__setattr__(self, name, value)
        self.fields[name] = value


Value: TypeAlias = (
    None | bool | int | float | str | list["Value"] | dict[str, "Value"] | FixtureObject
)


ItemT = TypeVar("ItemT")


def selected(item: Mapping[str, ItemT], ids: list[str]) -> bool:
    identifier = item.get("id")
    return identifier in ids


class RequestUtils(ResponsesAPIRequestUtils):
    def rewrite(
        self,
        response: ResponsesAPIResponse | dict[str, Value],
        provider: str | None,
        metadata: dict[str, JsonValue] | None,
    ) -> None:
        _updated: ResponsesAPIResponse | dict[str, Value] = (
            self._update_responses_api_response_id_with_model_id(response, provider, metadata)
        )

    def restore(self, request_input: Value) -> Value:
        return self._restore_encrypted_content_item_ids_in_input(request_input)


_UTILS = RequestUtils()


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.levels: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.levels.append(record.levelname)


class Diagnostics:
    def __init__(self) -> None:
        self.logger = logging.getLogger("LiteLLM")
        self.capture = Capture()
        self.previous_level = logging.NOTSET
        self.previous_handlers: list[logging.Handler] = []

    def __enter__(self) -> Self:
        self.previous_handlers = list(self.logger.handlers)
        for handler in self.previous_handlers:
            self.logger.removeHandler(handler)
        self.previous_level = self.logger.level
        self.logger.setLevel(logging.DEBUG)
        self.logger.addHandler(self.capture)
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        self.logger.removeHandler(self.capture)
        self.logger.setLevel(self.previous_level)
        for handler in self.previous_handlers:
            self.logger.addHandler(handler)

    def take(self) -> str | None:
        reported = "debug" if self.capture.levels else None
        self.capture.levels.clear()
        return reported


class RewriteScenario(TypedDict):
    response: dict[str, JsonValue]
    custom_llm_provider: NotRequired[str | None]
    litellm_metadata: NotRequired[dict[str, JsonValue] | None]


class StripScenario(TypedDict):
    input: JsonValue
    strip_ids: NotRequired[list[str] | None]


class ResponseRewriteInput(TypedDict):
    operation: Literal["response_rewrite"]
    scenarios: list[RewriteScenario]


class RestoreInput(TypedDict):
    operation: Literal["input_restore"]
    inputs: list[JsonValue]


class StripInput(TypedDict):
    operation: Literal["reasoning_strip"]
    scenarios: list[StripScenario]


class CollectionInput(TypedDict):
    operation: Literal["container_collection"]
    responses: list[dict[str, JsonValue] | None]


Input: TypeAlias = ResponseRewriteInput | RestoreInput | StripInput | CollectionInput


def object_from_wire(value: JsonValue) -> FixtureObject:
    parsed = TypeAdapter(ObjectWire).validate_python(value)
    fields = {name: from_wire(item) for name, item in parsed["fields"].items()}
    return FixtureObject(fields, parsed["readonly"])


def from_wire(wire: JsonValue) -> Value:
    match wire:
        case None | bool() | int() | float() | str():
            return wire
        case list():
            return [from_wire(item) for item in wire]
        case {"__object__": object_value}:
            return object_from_wire(object_value)
        case dict():
            return {name: from_wire(item) for name, item in wire.items()}


def object_to_wire(value: FixtureObject) -> JsonValue:
    fields: dict[str, JsonValue] = {
        name: to_wire(item) for name, item in value.fields.items()
    }
    readonly: list[JsonValue] = list[JsonValue](sorted(value.readonly))
    payload: dict[str, JsonValue] = {"fields": fields, "readonly": readonly}
    return {"__object__": payload}


def to_wire(value: Value) -> JsonValue:
    match value:
        case FixtureObject():
            return object_to_wire(value)
        case dict():
            return {name: to_wire(item) for name, item in value.items()}
        case list():
            return [to_wire(item) for item in value]
        case None | bool() | int() | float() | str():
            return value


def rewrite_scenario(scenario: RewriteScenario) -> JsonValue:
    response: dict[str, Value] = {
        name: from_wire(item) for name, item in scenario["response"].items()
    }
    with Diagnostics() as diagnostics:
        _UTILS.rewrite(
            response,
            scenario.get("custom_llm_provider"),
            scenario.get("litellm_metadata"),
        )
        diagnostic = diagnostics.take()
    return {"response": to_wire(response), "diagnostic": diagnostic}


def response_rewrite(input_value: ResponseRewriteInput) -> JsonValue:
    results = [rewrite_scenario(scenario) for scenario in input_value["scenarios"]]
    return {"scenarios": results}


def restore_input(wire: JsonValue) -> JsonValue:
    target = from_wire(wire)
    with Diagnostics() as diagnostics:
        _UTILS.restore(target)
        diagnostic = diagnostics.take()
    return {"input": to_wire(target), "diagnostic": diagnostic}


def input_restore(input_value: RestoreInput) -> JsonValue:
    results = [restore_input(item) for item in input_value["inputs"]]
    return {"inputs": results}


def strip_scenario(scenario: StripScenario) -> JsonValue:
    target = from_wire(scenario["input"])
    strip_ids = scenario.get("strip_ids")
    with Diagnostics() as diagnostics:
        if strip_ids is None:
            ResponsesAPIRequestUtils.strip_encrypted_reasoning_from_input(target)
        else:
            ResponsesAPIRequestUtils.strip_encrypted_reasoning_from_input(
                target, should_strip=partial(selected, ids=strip_ids)
            )
        diagnostic = diagnostics.take()
    return {"input": to_wire(target), "diagnostic": diagnostic}


def reasoning_strip(input_value: StripInput) -> JsonValue:
    results = [strip_scenario(scenario) for scenario in input_value["scenarios"]]
    return {"scenarios": results}


def collection_result(wire: dict[str, JsonValue] | None) -> JsonValue:
    response: dict[str, Value] | None = (
        None if wire is None else {name: from_wire(item) for name, item in wire.items()}
    )
    with Diagnostics() as diagnostics:
        ids = ResponsesAPIRequestUtils.collect_container_ids_from_responses_response(response)
        diagnostic = diagnostics.take()
    projected: JsonValue = None if response is None else to_wire(response)
    result: dict[str, JsonValue] = {
        "ids": list[JsonValue](sorted(ids)),
        "response": projected,
        "diagnostic": diagnostic,
    }
    return result


def container_collection(input_value: CollectionInput) -> JsonValue:
    results = [collection_result(response) for response in input_value["responses"]]
    return {"responses": results}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "response_rewrite":
        return response_rewrite(input_value)
    if input_value["operation"] == "input_restore":
        return input_restore(input_value)
    if input_value["operation"] == "reasoning_strip":
        return reasoning_strip(input_value)
    if input_value["operation"] == "container_collection":
        return container_collection(input_value)
    raise ValueError(f"unsupported response_id_traversal operation: {input_value['operation']}")
