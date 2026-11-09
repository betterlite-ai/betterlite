import json
from collections.abc import Callable, Mapping, Sequence
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.arize import _utils
from litellm.integrations.langtrace import LangtraceAttributes
from litellm.integrations.otel.mappers import (
    genai,
    langfuse,
    langtrace,
    legacy,
    openinference,
    resolve_mappers,
    utils,
    weave,
)
from litellm.integrations.otel.mappers.base import AttributeMapper
from litellm.integrations.otel.model import payloads
from litellm.integrations.otel.model.trace_controls import TraceControls

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
MapperClass: TypeAlias = Callable[..., AttributeMapper]

set_langtrace_attributes = cast(
    Callable[[object, dict[str, Json], dict[str, Json]], None],
    cast(object, LangtraceAttributes().set_langtrace_attributes),
)
infer_span_kind = cast(
    Callable[[str | None], str],
    cast(object, _utils._infer_open_inference_span_kind),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)
set_tool_attributes = cast(
    Callable[[object, list[Json] | None, list[Json] | None], None],
    cast(object, _utils._set_tool_attributes),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
)


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict[str, Json] = {}
        self.events: list[Json] = []

    def set_attribute(self, key: str, value: Json) -> None:
        self.attributes[key] = json_value(value)

    def add_event(self, name: str, attributes: dict[str, Json] | None = None) -> None:
        recorded: dict[str, Json] = {} if attributes is None else {
            key: json_value(value) for key, value in attributes.items()
        }
        self.events.append({"name": name, "attributes": recorded})


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    result: Json
    if function == "langfuse_llm_call":
        result = plain_mapper_record(input_value, langfuse.LangfuseMapper)
    elif function == "langtrace_llm_call":
        result = plain_mapper_record(input_value, langtrace.LangtraceMapper)
    elif function == "weave_llm_call":
        result = plain_mapper_record(input_value, weave.WeaveMapper)
    elif function == "openinference_llm_call":
        result = tool_mapper_record(input_value, openinference.OpenInferenceMapper)
    elif function == "openinference_message_shedding":
        result = shedding_record(input_value)
    elif function == "mapper_registry":
        result = registry_record(input_value)
    elif function == "langtrace_integration":
        result = integration_record(input_value)
    elif function == "infer_span_kind":
        result = {"kind": infer_span_kind(optional_string(input_value.get("call_type")))}
    elif function == "tool_attributes":
        result = tool_attributes_record(input_value)
    else:
        raise ValueError(f"unknown b3846 function {function} ({litellm.__name__})")
    return result


def plain_mapper_record(input_value: dict[str, Json], mapper_class: MapperClass) -> Json:
    mapped = mapper_class().map(llm_call_span(input_value))
    return attributes_record(cast(Mapping[str, object], mapped))


def tool_mapper_record(input_value: dict[str, Json], mapper_class: MapperClass) -> Json:
    mapper = mapper_class(tool_attr_budget=tool_budget_value(input_value))
    return attributes_record(cast(Mapping[str, object], mapper.map(llm_call_span(input_value))))


def shedding_record(input_value: dict[str, Json]) -> Json:
    mapper = openinference.OpenInferenceMapper(
        tool_attr_budget=tool_budget_value(input_value)
    )
    mapped = mapper.map(llm_call_span(input_value))
    budget = optional_integer(input_value.get("budget"))
    fitted = openinference.fit_indexed_messages(mapped, budget)
    return attributes_record(cast(Mapping[str, object], fitted))


def registry_record(input_value: dict[str, Json]) -> Json:
    names = [string_value(item, "name") for item in list_value(input_value.get("names"), "names")]
    return [mapper_entry(mapper) for mapper in resolve_mappers(names)]


MAPPER_NAMES: dict[type, str] = {
    genai.GenAIMapper: "genai",
    legacy.LegacyMapper: "legacy",
    langfuse.LangfuseMapper: "langfuse",
    langtrace.LangtraceMapper: "langtrace",
    openinference.OpenInferenceMapper: "openinference",
    weave.WeaveMapper: "weave",
}


def mapper_entry(mapper: AttributeMapper) -> Json:
    budget: Json = None
    if isinstance(
        mapper,
        (genai.GenAIMapper, legacy.LegacyMapper, openinference.OpenInferenceMapper),
    ):
        budget = mapper._tool_attr_budget  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"mapper": MAPPER_NAMES[type(mapper)], "tool_attr_budget": budget}


def integration_record(input_value: dict[str, Json]) -> Json:
    kwargs = object_value(input_value.get("kwargs"), "kwargs")
    response = object_value(input_value.get("response"), "response")
    span = RecordingSpan()
    set_langtrace_attributes(span, kwargs, response)
    return cast(Json, {"attributes": span.attributes, "events": span.events})


def tool_attributes_record(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    set_tool_attributes(
        span,
        optional_list(input_value.get("optional_tools")),
        optional_list(input_value.get("metadata_tools")),
    )
    return span.attributes


def llm_call_span(input_value: dict[str, Json]) -> payloads.LLMCallSpanData:
    payload = object_value(input_value.get("payload"), "payload")
    return payloads.LLMCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=bool_value(input_value.get("capture_content")),
        time_to_first_chunk_seconds=optional_float(
            input_value.get("time_to_first_chunk_seconds")
        ),
        request_route=optional_string(input_value.get("request_route")),
        trace=trace_controls(input_value.get("trace")),
        session_id=optional_string(input_value.get("session_id")),
    )


def trace_controls(value: Json) -> TraceControls | None:
    if value is None:
        return None
    fields = object_value(value, "trace")
    return TraceControls(
        name=optional_string(fields.get("name")),
        user_id=optional_string(fields.get("user_id")),
        session_id=optional_string(fields.get("session_id")),
        tags=tuple(string_value(tag, "tag") for tag in list_value(fields.get("tags", []), "tags")),
    )


def tool_budget_value(input_value: dict[str, Json]) -> int:
    raw = input_value.get("tool_attr_budget")
    return (
        utils.MAX_TOOL_DEFINITION_ATTRS_PER_SPAN
        if raw is None
        else integer_value(raw, "tool_attr_budget")
    )


def attributes_record(attributes: Mapping[str, object]) -> Json:
    return cast(Json, {key: json_value(value) for key, value in attributes.items()})


def json_value(value: object) -> Json:
    if isinstance(value, str):
        try:
            parsed: Json = cast(Json, json.loads(value))
        except json.JSONDecodeError:
            return value
        return parsed
    if value is None or isinstance(value, (bool, int, float)):
        return value
    if isinstance(value, (list, tuple)):
        return cast(Json, [json_value(item) for item in cast(Sequence[object], value)])
    raise ValueError(f"unsupported attribute value of type {type(value).__name__}")


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def optional_list(value: Json) -> list[Json] | None:
    return None if value is None else list_value(value, "tools")


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_integer(value: Json) -> int | None:
    return None if value is None else integer_value(value, "budget")


def bool_value(value: Json) -> bool:
    if value is None:
        return False
    if not isinstance(value, bool):
        raise ValueError("capture_content must be a boolean")
    return value


def optional_float(value: Json) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("time_to_first_chunk_seconds must be a number")
    return float(value)


def optional_string(value: Json) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("expected a string")
    return value
