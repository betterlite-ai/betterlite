import json
from collections.abc import Callable, Mapping
from types import SimpleNamespace
from typing import Protocol, TypeAlias, cast

import litellm
from litellm.integrations.arize import _utils
from litellm.integrations.weave import weave_otel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
AttributesClass: TypeAlias = type[object]
JSON_ATTRIBUTES = frozenset(
    {"input.value", "output.value", "metadata", "llm.invocation_parameters"}
)


class AttributeSpan(Protocol):
    def set_attribute(self, key: str, value: Json) -> None: ...

    def record_exception(self, exception: BaseException) -> None: ...


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict[str, Json] = {}
        self.exceptions: list[Json] = []

    def set_attribute(self, key: str, value: Json) -> None:
        self.attributes[key] = value

    def record_exception(self, exception: BaseException) -> None:
        self.exceptions.append({"type": type(exception).__name__, "message": str(exception)})


set_attributes = cast(
    Callable[[AttributeSpan, dict[str, Json], Json, AttributesClass], None],
    cast(object, _utils.set_attributes),
)
set_weave_otel_attributes = cast(
    Callable[[AttributeSpan, Mapping[str, object], object], None],
    cast(object, weave_otel.set_weave_otel_attributes),
)
arize_attributes = cast(AttributesClass, cast(object, _utils.ArizeOTELAttributes))


def run(input_value: dict[str, Json]) -> Json:
    match input_value.get("function"):
        case "set_attributes":
            return {"result": arize_record(input_value)}
        case "set_weave_attributes":
            return {"result": weave_record(input_value)}
        case other:
            raise ValueError(f"unknown b3861 function {other} ({litellm.__name__})")


def arize_record(input_value: dict[str, Json]) -> Json:
    kwargs = object_value(input_value.get("kwargs"), "kwargs")
    prepare_redaction(kwargs, input_value.get("redact_messages"))
    span = RecordingSpan()
    set_attributes(span, kwargs, response_value(input_value.get("response")), arize_attributes)
    return result_record(span, kwargs)


def weave_record(input_value: dict[str, Json]) -> Json:
    kwargs = object_value(input_value.get("kwargs"), "kwargs")
    prepare_redaction(kwargs, input_value.get("redact_messages"))
    span = RecordingSpan()
    set_weave_otel_attributes(span, kwargs, response_value(input_value.get("response")))
    return result_record(span, kwargs)


def prepare_redaction(kwargs: dict[str, Json], redact_messages: Json) -> None:
    if not redact_messages:
        return
    litellm_params = mapping_field(kwargs, "litellm_params")
    metadata = mapping_field(litellm_params, "metadata")
    headers = mapping_field(metadata, "headers")
    headers["x-litellm-enable-message-redaction"] = True


def mapping_field(container: dict[str, Json], key: str) -> dict[str, Json]:
    value = container.get(key)
    if isinstance(value, dict):
        return value
    created: dict[str, Json] = {}
    container[key] = created
    return created


def result_record(span: RecordingSpan, kwargs: dict[str, Json]) -> Json:
    attributes = normalize_attributes(span.attributes)
    drop_array_contents(attributes, kwargs.get("messages"))
    return {"attributes": attributes, "exceptions": list(span.exceptions)}


def normalize_attributes(attributes: dict[str, Json]) -> dict[str, Json]:
    normalized: dict[str, Json] = {}
    for key, value in attributes.items():
        if key in JSON_ATTRIBUTES and isinstance(value, str):
            parsed_ok, parsed = parse_json(value)
            normalized[key] = parsed if parsed_ok else value
        else:
            normalized[key] = value
    return normalized


def drop_array_contents(attributes: dict[str, Json], messages: Json) -> None:
    if not isinstance(messages, list):
        return
    last = messages[-1] if messages else None
    if isinstance(last, dict) and isinstance(last.get("content"), list):
        attributes.pop("input.value", None)
    for index, message in enumerate(messages):
        if isinstance(message, dict) and isinstance(message.get("content"), list):
            attributes.pop(f"llm.input_messages.{index}.message.content", None)


def parse_json(text: str) -> tuple[bool, Json]:
    try:
        return True, cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return False, text


def response_value(response: Json) -> Json:
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


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value
