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

MAPPER_NAMES: dict[type, str] = {
    genai.GenAIMapper: "genai",
    legacy.LegacyMapper: "legacy",
    langfuse.LangfuseMapper: "langfuse",
    langtrace.LangtraceMapper: "langtrace",
    openinference.OpenInferenceMapper: "openinference",
    weave.WeaveMapper: "weave",
}


class RecordingSpan:
    def __init__(self) -> None:
        self.attributes: dict[str, Json] = {}
        self.events: list[Json] = []

    def set_attribute(self, key: str, value: Json) -> None:
        self.attributes[key] = json_value(value)

    def add_event(self, name: str, attributes: dict[str, Json] | None = None) -> None:
        recorded = (
            {}
            if attributes is None
            else {key: json_value(value) for key, value in attributes.items()}
        )
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
        result = {"kind": infer_span_kind(cast(str | None, input_value.get("call_type")))}
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
    mapped = mapper.map(llm_call_span(input_value))
    return attributes_record(cast(Mapping[str, object], mapped))


def shedding_record(input_value: dict[str, Json]) -> Json:
    mapper = openinference.OpenInferenceMapper(tool_attr_budget=tool_budget_value(input_value))
    mapped = mapper.map(llm_call_span(input_value))
    budget = cast(int | None, input_value.get("budget"))
    fitted = openinference.fit_indexed_messages(mapped, budget)
    return attributes_record(cast(Mapping[str, object], fitted))


def registry_record(input_value: dict[str, Json]) -> Json:
    names = cast(list[str], input_value.get("names"))
    return [mapper_entry(mapper) for mapper in resolve_mappers(names)]


def mapper_entry(mapper: AttributeMapper) -> Json:
    budget: Json = None
    if isinstance(
        mapper,
        (genai.GenAIMapper, legacy.LegacyMapper, openinference.OpenInferenceMapper),
    ):
        budget = mapper._tool_attr_budget  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"mapper": MAPPER_NAMES[type(mapper)], "tool_attr_budget": budget}


def integration_record(input_value: dict[str, Json]) -> Json:
    kwargs = cast(dict[str, Json], input_value.get("kwargs"))
    response = cast(dict[str, Json], input_value.get("response"))
    span = RecordingSpan()
    set_langtrace_attributes(span, kwargs, response)
    return cast(Json, {"attributes": span.attributes, "events": span.events})


def tool_attributes_record(input_value: dict[str, Json]) -> Json:
    span = RecordingSpan()
    set_tool_attributes(
        span,
        cast(list[Json] | None, input_value.get("optional_tools")),
        cast(list[Json] | None, input_value.get("metadata_tools")),
    )
    return span.attributes


def llm_call_span(input_value: dict[str, Json]) -> payloads.LLMCallSpanData:
    payload = cast(dict[str, Json], input_value.get("payload"))
    return payloads.LLMCallSpanData.from_standard_logging_payload(
        cast("StandardLoggingPayload", payload),
        capture_content=cast(bool, input_value.get("capture_content", False)),
        time_to_first_chunk_seconds=cast(
            float | None, input_value.get("time_to_first_chunk_seconds")
        ),
        request_route=cast(str | None, input_value.get("request_route")),
        trace=trace_controls(input_value.get("trace")),
        session_id=cast(str | None, input_value.get("session_id")),
    )


def trace_controls(value: Json) -> TraceControls | None:
    if value is None:
        return None
    fields = cast(dict[str, Json], value)
    return TraceControls(
        name=cast(str | None, fields.get("name")),
        user_id=cast(str | None, fields.get("user_id")),
        session_id=cast(str | None, fields.get("session_id")),
        tags=tuple(cast(list[str], fields.get("tags", []))),
    )


def tool_budget_value(input_value: dict[str, Json]) -> int:
    return cast(
        int,
        input_value.get("tool_attr_budget", utils.MAX_TOOL_DEFINITION_ATTRS_PER_SPAN),
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
