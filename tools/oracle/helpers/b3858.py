import asyncio
import json
from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Literal, NotRequired, Protocol, TypedDict, TypeAlias

from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import ManagementEndpointLoggingPayload, UserAPIKeyAuth
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from litellm.types.utils import LLMResponseTypes, ModelResponse
from opentelemetry.sdk.trace import Event, ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Span
from opentelemetry.util.types import AttributeValue
from pydantic import JsonValue, TypeAdapter, ValidationError

PARENT_SPAN_KEY = "litellm_parent_otel_span"
DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class Parent(TypedDict):
    name: str
    start: float
    end: NotRequired[float]


class ExceptionSpec(TypedDict):
    kind: Literal["authentication", "value_error"]
    message: str


class Team(TypedDict):
    id: NotRequired[str]
    alias: NotRequired[str]


class Common(TypedDict):
    start_time: float
    end_time: float
    parent: NotRequired[Parent]


class ServiceFields(Common):
    function: Literal["service_success", "service_failure"]
    service: str
    call_type: str
    caller: NotRequired[str | None]
    error: NotRequired[str | None]
    event_metadata: NotRequired[dict[str, JsonValue] | None]


class ServiceSuccess(ServiceFields):
    function: Literal["service_success"]


class ServiceFailure(ServiceFields):
    function: Literal["service_failure"]


class PostCallFailure(Common):
    function: Literal["post_call_failure"]
    kwargs: dict[str, JsonValue]
    request_data: dict[str, JsonValue]
    exception: ExceptionSpec
    team: NotRequired[Team]
    after_failure_log: NotRequired[bool]


class PostCallSuccess(Common):
    function: Literal["post_call_success"]
    kwargs: dict[str, JsonValue]


class ManagementFields(Common):
    function: Literal["management_success", "management_failure"]
    route: str
    request_data: dict[str, JsonValue]
    response: NotRequired[dict[str, JsonValue]]
    exception: NotRequired[ExceptionSpec]


class ManagementSuccess(ManagementFields):
    function: Literal["management_success"]


class ManagementFailure(ManagementFields):
    function: Literal["management_failure"]


class ErrorAttributes(Common):
    function: Literal["error_attributes"]
    status_code: int
    exception: NotRequired[ExceptionSpec]


Scenario: TypeAlias = (
    ServiceSuccess
    | ServiceFailure
    | PostCallFailure
    | PostCallSuccess
    | ManagementSuccess
    | ManagementFailure
    | ErrorAttributes
)

# the parent span travels as a live Span object, so one metadata level holds a non-JSON value
CarriedMetadata: TypeAlias = dict[str, JsonValue | Span]
CarriedParams: TypeAlias = dict[str, JsonValue | CarriedMetadata]
CarriedKwargs: TypeAlias = dict[str, JsonValue | CarriedMetadata | CarriedParams]
CarriedRequest: TypeAlias = dict[str, JsonValue | CarriedMetadata]


class EventLine(TypedDict):
    name: str
    attributes: dict[str, str]


class SpanLine(TypedDict):
    name: str
    parent: str
    kind: str
    status: str
    start: str
    end: str
    attributes: dict[str, str]
    events: list[EventLine]


class Observed(TypedDict):
    spans: list[SpanLine]


class LegacyCallbacks(Protocol):
    async def async_service_success_hook(
        self,
        payload: ServiceLoggerPayload,
        parent_otel_span: Span | None = None,
        start_time: datetime | float | None = None,
        end_time: datetime | float | None = None,
        event_metadata: dict[str, JsonValue] | None = None,
    ) -> None: ...

    async def async_service_failure_hook(
        self,
        payload: ServiceLoggerPayload,
        error: str | None = "",
        parent_otel_span: Span | None = None,
        start_time: datetime | float | None = None,
        end_time: float | datetime | None = None,
        event_metadata: dict[str, JsonValue] | None = None,
    ) -> None: ...

    # ast-grep-ignore: no-long-names
    async def async_management_endpoint_success_hook(
        self,
        # ast-grep-ignore: no-long-names
        logging_payload: ManagementEndpointLoggingPayload,
        parent_otel_span: Span | None = None,
    ) -> None: ...

    # ast-grep-ignore: no-long-names
    async def async_management_endpoint_failure_hook(
        self,
        # ast-grep-ignore: no-long-names
        logging_payload: ManagementEndpointLoggingPayload,
        parent_otel_span: Span | None = None,
    ) -> None: ...

    async def async_post_call_failure_hook(
        self,
        request_data: CarriedRequest,
        original_exception: Exception,
        user_api_key_dict: UserAPIKeyAuth,
        traceback_str: str | None = None,
    ) -> None: ...

    async def async_post_call_success_hook(
        self,
        data: dict[str, Logging],
        user_api_key_dict: UserAPIKeyAuth,
        response: LLMResponseTypes,
    ) -> None: ...

    async def async_log_failure_event(
        self,
        kwargs: CarriedKwargs,
        response_obj: None,
        start_time: datetime,
        end_time: datetime,
    ) -> None: ...

    # ast-grep-ignore: no-long-names
    def record_error_attributes_on_span(
        self,
        span: Span | None,
        exception: Exception | None,
        status_code: int,
    ) -> None: ...


class AuthenticationError(Exception):
    # the OTel SDK puts the class module into exception.type unless it is builtins
    __module__ = "builtins"
    status_code: int = 401
    llm_provider: str = "openai"
    message: str

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = f"litellm.AuthenticationError: {message}"

    def __str__(self) -> str:
        return self.message


def exception_of(spec: ExceptionSpec) -> Exception:
    if spec["kind"] == "authentication":
        return AuthenticationError(spec["message"])
    return ValueError(spec["message"])


def common_parent(input_value: Common) -> Parent | None:
    if "parent" in input_value:
        return input_value["parent"]
    return None


def parent_span(provider: TracerProvider, spec: Parent | None) -> Span | None:
    if spec is None:
        return None
    tracer = provider.get_tracer("b3858")
    return tracer.start_span(spec["name"], start_time=int(spec["start"] * 1e9))


def end_parent(parent: Span | None, spec: Parent | None, end_time: float) -> None:
    if parent is None or not parent.is_recording():
        return
    end = end_time
    if spec is not None and "end" in spec:
        end = spec["end"]
    parent.end(end_time=int(end * 1e9))


def metadata_bucket(fields: Mapping[str, JsonValue]) -> str:
    if "litellm_metadata" in fields:
        return "litellm_metadata"
    return "metadata"


def with_carrier(kwargs: dict[str, JsonValue], parent: Span) -> CarriedKwargs:
    params = DICT.validate_python(kwargs["litellm_params"])
    metadata = DICT.validate_python(params["metadata"])
    carried_metadata: CarriedMetadata = {**metadata, PARENT_SPAN_KEY: parent}
    carried_params: CarriedParams = {**params, "metadata": carried_metadata}
    return {**kwargs, "litellm_params": carried_params}


def failure_request(
    input_value: PostCallFailure, parent: Span | None
) -> tuple[CarriedKwargs, CarriedRequest]:
    request_data = DICT.validate_python(input_value["request_data"])
    bucket = metadata_bucket(request_data)
    metadata = DICT.validate_python(request_data[bucket])
    carried_metadata: CarriedMetadata = metadata if parent is None else {**metadata, PARENT_SPAN_KEY: parent}
    request_data[bucket] = carried_metadata
    params = DICT.validate_python(input_value["kwargs"]["litellm_params"])
    carried_params: CarriedParams = {**params, "metadata": carried_metadata}
    carried_kwargs: CarriedKwargs = {**input_value["kwargs"], "litellm_params": carried_params}
    return carried_kwargs, request_data


def proxy_key(input_value: PostCallFailure, parent: Span | None) -> UserAPIKeyAuth:
    team = input_value.get("team")
    if team is None:
        return UserAPIKeyAuth(parent_otel_span=parent)
    return UserAPIKeyAuth(
        parent_otel_span=parent,
        team_id=team.get("id"),
        team_alias=team.get("alias"),
    )


def service_payload(input_value: ServiceFields) -> ServiceLoggerPayload:
    is_error = input_value["function"] == "service_failure"
    return ServiceLoggerPayload(
        is_error=is_error,
        service=ServiceTypes(input_value["service"]),
        duration=1.0,
        call_type=input_value["call_type"],
        caller=input_value.get("caller"),
        event_metadata=None,
    )


def service_call(dispatch: LegacyCallbacks, input_value: ServiceFields, parent: Span | None) -> None:
    payload = service_payload(input_value)
    if input_value["function"] == "service_success":
        asyncio.run(
            dispatch.async_service_success_hook(
                payload,
                parent,
                input_value["start_time"],
                input_value["end_time"],
                input_value.get("event_metadata"),
            )
        )
        return
    asyncio.run(
        dispatch.async_service_failure_hook(
            payload,
            input_value.get("error"),
            parent,
            input_value["start_time"],
            input_value["end_time"],
            input_value.get("event_metadata"),
        )
    )


def management_payload(input_value: ManagementFields) -> ManagementEndpointLoggingPayload:
    exception = input_value.get("exception")
    return ManagementEndpointLoggingPayload(
        route=input_value["route"],
        request_data=DICT.validate_python(input_value["request_data"]),
        response=input_value.get("response"),
        exception=None if exception is None else exception_of(exception),
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        end_time=datetime.fromtimestamp(input_value["end_time"], tz=UTC),
    )


def management_call(
    dispatch: LegacyCallbacks, input_value: ManagementFields, parent: Span | None
) -> None:
    payload = management_payload(input_value)
    if input_value["function"] == "management_success":
        asyncio.run(dispatch.async_management_endpoint_success_hook(payload, parent))
        return
    asyncio.run(dispatch.async_management_endpoint_failure_hook(payload, parent))


def success_call(dispatch: LegacyCallbacks, input_value: PostCallSuccess, parent: Span | None) -> None:
    details = input_value["kwargs"]
    carried = details if parent is None else with_carrier(details, parent)
    logging_obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    logging_obj.model_call_details = carried
    data: dict[str, Logging] = {"litellm_logging_obj": logging_obj}
    response = ModelResponse(id="chatcmpl-b3858", choices=[], created=1_704_067_200)
    asyncio.run(
        dispatch.async_post_call_success_hook(data, UserAPIKeyAuth(parent_otel_span=parent), response)
    )


def failure_call(dispatch: LegacyCallbacks, input_value: PostCallFailure, parent: Span | None) -> None:
    exception = exception_of(input_value["exception"])
    kwargs, request_data = failure_request(input_value, parent)
    if input_value.get("after_failure_log") is True:
        start = datetime.fromtimestamp(input_value["start_time"], tz=UTC)
        end = datetime.fromtimestamp(input_value["end_time"], tz=UTC)
        asyncio.run(dispatch.async_log_failure_event(kwargs, None, start, end))
    asyncio.run(
        dispatch.async_post_call_failure_hook(
            request_data,
            exception,
            proxy_key(input_value, parent),
            None,
        )
    )


def error_attributes_call(
    dispatch: LegacyCallbacks, input_value: ErrorAttributes, parent: Span | None
) -> None:
    spec = input_value.get("exception")
    exception = None if spec is None else exception_of(spec)
    dispatch.record_error_attributes_on_span(parent, exception, input_value["status_code"])


def json_text(text: str) -> str:
    if not text.startswith(("{", "[")):
        return text
    try:
        parsed = JSON.validate_json(text)
    except ValidationError:
        return text
    return json.dumps(parsed, sort_keys=True, separators=(",", ":"))


def value_text(value: JsonValue) -> str:
    match value:
        case None | bool() as flag:
            return "true" if flag else "false"
        case str():
            return json_text(value)
        case int() | float():
            return str(value)
        case list():
            return ",".join(value_text(item) for item in value)
        case dict():
            return json.dumps(value, sort_keys=True, separators=(",", ":"))


def attribute_map(attributes: Mapping[str, AttributeValue] | None) -> dict[str, str]:
    if attributes is None:
        return {}
    values = DICT.validate_python(attributes)
    return {key: value_text(value) for key, value in values.items()}


def event_line(event: Event) -> EventLine:
    return EventLine(name=event.name, attributes=attribute_map(event.attributes))


def line_of(span: ReadableSpan, names: Mapping[int, str]) -> SpanLine:
    parent_context = span.parent
    parent_id = 0 if parent_context is None else parent_context.span_id
    parent_name = ""
    if parent_id in names:
        parent_name = names[parent_id]
    return SpanLine(
        name=span.name,
        parent=parent_name,
        kind=span.kind.name,
        status=span.status.status_code.name,
        start=str(span.start_time),
        end=str(span.end_time),
        attributes=attribute_map(span.attributes),
        events=[event_line(event) for event in span.events],
    )


def read_spans(exporter: InMemorySpanExporter) -> list[SpanLine]:
    spans = exporter.get_finished_spans()
    names: dict[int, str] = {}
    for span in spans:
        context = span.get_span_context()
        if context is not None:
            names[context.span_id] = span.name
    lines = [line_of(span, names) for span in spans]
    rows: list[tuple[int, int, str, str, int]] = [
        (int(line["start"]), int(line["end"]), line["name"], line["parent"], index)
        for index, line in enumerate(lines)
    ]
    rows.sort()
    return [lines[row[4]] for row in rows]


def run(input_value: Scenario) -> JsonValue:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    logger = OpenTelemetry(
        config=OpenTelemetryConfig(exporter="in_memory", skip_set_global=True),
        tracer_provider=provider,
    )
    spec = common_parent(input_value)
    parent = parent_span(provider, spec)
    if input_value["function"] == "service_success":
        service_call(logger, input_value, parent)
    elif input_value["function"] == "service_failure":
        service_call(logger, input_value, parent)
    elif input_value["function"] == "post_call_failure":
        failure_call(logger, input_value, parent)
    elif input_value["function"] == "post_call_success":
        success_call(logger, input_value, parent)
    elif input_value["function"] == "management_success":
        management_call(logger, input_value, parent)
    elif input_value["function"] == "management_failure":
        management_call(logger, input_value, parent)
    else:
        error_attributes_call(logger, input_value, parent)
    end_parent(parent, spec, input_value["end_time"])
    return JSON.validate_python(Observed(spans=read_spans(exporter)))
