import asyncio
from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict

from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import ManagementEndpointLoggingPayload, UserAPIKeyAuth
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from litellm.types.utils import ModelResponse
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter

from b3859.observe import attribute_map

DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
Carried = dict[str, JsonValue | Span | "Carried"]


class Parent(TypedDict):
    name: str
    start: NotRequired[float]
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


class ErrorAttributes(Common):
    function: Literal["error_attributes"]
    status_code: int
    exception: NotRequired[ExceptionSpec]


Scenario = ServiceFields | PostCallFailure | PostCallSuccess | ManagementFields | ErrorAttributes


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


class AuthenticationError(Exception):
    __module__ = "builtins"
    status_code: int = 401
    llm_provider: str = "openai"

    def __init__(self, message: str) -> None:
        super().__init__(f"litellm.AuthenticationError: {message}")


def exception_of(spec: ExceptionSpec) -> Exception:
    if spec["kind"] == "authentication":
        return AuthenticationError(spec["message"])
    return ValueError(spec["message"])


def with_carrier(kwargs: dict[str, JsonValue], parent: Span) -> Carried:
    params = DICT.validate_python(kwargs["litellm_params"])
    metadata = DICT.validate_python(params["metadata"])
    carried: Carried = {**metadata, "litellm_parent_otel_span": parent}
    return {**kwargs, "litellm_params": {**params, "metadata": carried}}


def failure_request(
    input_value: PostCallFailure, parent: Span | None
) -> tuple[Carried, dict[str, JsonValue]]:
    params = DICT.validate_python(input_value["kwargs"]["litellm_params"])
    metadata: Carried = {**DICT.validate_python(params["metadata"])}
    if parent is not None:
        metadata["litellm_parent_otel_span"] = parent
    kwargs: Carried = {**input_value["kwargs"], "litellm_params": {**params, "metadata": metadata}}
    return kwargs, input_value["request_data"]


async def service_call(
    logger: OpenTelemetry,
    input_value: ServiceFields,
    parent: Span | None,
    *,
    mode: Literal["success", "failure"],
) -> None:
    payload = ServiceLoggerPayload(
        is_error=mode == "failure", error=None, service=ServiceTypes(input_value["service"]),
        duration=1.0, call_type=input_value["call_type"], caller=input_value.get("caller"),
        event_metadata=None,
    )
    if mode == "success":
        await logger.async_service_success_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L792
            payload,
            parent,
            input_value["start_time"],
            input_value["end_time"],
            input_value.get("event_metadata"),
        )
        return
    await logger.async_service_failure_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L845
        payload=payload,
        error=input_value.get("error"),
        parent_otel_span=parent,
        start_time=input_value["start_time"],
        end_time=input_value["end_time"],
        event_metadata=input_value.get("event_metadata"),
    )


async def management_call(
    logger: OpenTelemetry,
    input_value: ManagementFields,
    parent: Span | None,
    *,
    mode: Literal["success", "failure"],
) -> None:
    exception = input_value.get("exception")
    payload = ManagementEndpointLoggingPayload(
        route=input_value["route"],
        request_data=DICT.validate_python(input_value["request_data"]),
        response=input_value.get("response"),
        exception=None if exception is None else exception_of(exception),
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        end_time=datetime.fromtimestamp(input_value["end_time"], tz=UTC),
    )
    if mode == "success":
        await logger.async_management_endpoint_success_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L3382
            payload, parent
        )
        return
    await logger.async_management_endpoint_failure_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L3440
        payload, parent
    )


async def success_call(
    logger: OpenTelemetry, input_value: PostCallSuccess, parent: Span | None
) -> None:
    kwargs = input_value["kwargs"]
    details: Carried = {**kwargs} if parent is None else with_carrier(kwargs, parent)
    logging_obj = Logging(
        model="gpt-4o", messages=None, stream=False, call_type="completion",
        start_time=datetime.fromtimestamp(input_value["start_time"], tz=UTC),
        litellm_call_id="call-1", function_id="fn-1",
    )
    logging_obj.model_call_details = details
    data: dict[str, Logging] = {"litellm_logging_obj": logging_obj}
    await logger.async_post_call_success_hook(
        data, UserAPIKeyAuth(parent_otel_span=parent), ModelResponse.model_construct()
    )


async def failure_call(
    logger: OpenTelemetry, input_value: PostCallFailure, parent: Span | None
) -> None:
    kwargs, request_data = failure_request(input_value, parent)
    if input_value.get("after_failure_log") is True:
        start = datetime.fromtimestamp(input_value["start_time"], tz=UTC)
        end = datetime.fromtimestamp(input_value["end_time"], tz=UTC)
        await logger.async_log_failure_event(  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L765
            kwargs, None, start, end
        )
    team = input_value.get("team")
    key = UserAPIKeyAuth(parent_otel_span=parent)
    if team is not None:
        key = UserAPIKeyAuth(
            parent_otel_span=parent, team_id=team.get("id"), team_alias=team.get("alias")
        )
    await logger.async_post_call_failure_hook(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L903
        request_data, exception_of(input_value["exception"]), key, None
    )


def error_attributes_call(
    logger: OpenTelemetry, input_value: ErrorAttributes, parent: Span | None
) -> None:
    spec = input_value.get("exception")
    exception = None if spec is None else exception_of(spec)
    logger.record_error_attributes_on_span(parent, exception, input_value["status_code"])


def read_spans(exporter: InMemorySpanExporter) -> list[SpanLine]:
    spans = exporter.get_finished_spans()
    names: dict[int, str] = {}
    for span in spans:
        context = span.get_span_context()
        if context is not None:
            names[context.span_id] = span.name
    lines: list[SpanLine] = []
    for span in spans:
        parent = span.parent
        parent_id = 0 if parent is None else parent.span_id
        lines.append(
            SpanLine(
                name=span.name,
                parent=names[parent_id] if parent_id in names else "",
                kind=span.kind.name,
                status=span.status.status_code.name,
                start=str(span.start_time),
                end=str(span.end_time),
                attributes=attribute_map(span.attributes),
                events=[
                    EventLine(name=event.name, attributes=attribute_map(event.attributes))
                    for event in span.events
                ],
            )
        )
    lines.sort(key=lambda line: (int(line["start"]), int(line["end"]), line["name"]))
    return lines


def run(input_value: Scenario) -> JsonValue:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    logger = OpenTelemetry(
        config=OpenTelemetryConfig(exporter="in_memory", skip_set_global=True),
        tracer_provider=provider,
    )
    spec = input_value.get("parent")
    parent: Span | None = None
    if spec is not None:
        start = spec.get("start")
        start = input_value["start_time"] if start is None else start
        parent = provider.get_tracer("b3858").start_span(spec["name"], start_time=int(start * 1e9))
    match input_value["function"]:
        case "service_success":
            asyncio.run(service_call(logger, input_value, parent, mode="success"))
        case "service_failure":
            asyncio.run(service_call(logger, input_value, parent, mode="failure"))
        case "post_call_failure":
            asyncio.run(failure_call(logger, input_value, parent))
        case "post_call_success":
            asyncio.run(success_call(logger, input_value, parent))
        case "management_success":
            asyncio.run(management_call(logger, input_value, parent, mode="success"))
        case "management_failure":
            asyncio.run(management_call(logger, input_value, parent, mode="failure"))
        case "error_attributes":
            error_attributes_call(logger, input_value, parent)
    if parent is not None and spec is not None:
        end = spec.get("end")
        end = input_value["end_time"] if end is None else end
        parent.end(end_time=int(end * 1e9))
    return DICT.validate_python({"spans": read_spans(exporter)})
