import asyncio
from typing import Literal, TypedDict

import litellm
from litellm.integrations.otel.logger import OpenTelemetryV2, emit_guardrail_span, phase_span
from litellm.integrations.otel.model.config import OpenTelemetryV2Config
from litellm.integrations.otel.plumbing.context import set_request_root_span
from litellm.integrations.otel.plumbing.providers import build_tracer_provider
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from litellm.types.utils import StandardLoggingGuardrailInformation
from opentelemetry.sdk.trace import ReadableSpan, Tracer
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import INVALID_SPAN_ID, StatusCode

START: float = 1_704_067_200.0
END: float = 1_704_067_201.5


class Input(TypedDict):
    function: Literal["init", "registration", "service", "guardrail", "precall", "phase"]
    call_type: str
    error: str | None
    metrics_only: bool
    status: int
    second_status: int


class SpanRecord(TypedDict):
    name: str
    parent: Literal["root", "none", "other"]
    status: Literal["unset", "ok", "error"]
    events: int
    error_type: str | None
    error_message: str | None
    error_code: str | None
    service: str | None
    guardrail: str | None


class Answer(TypedDict):
    constructed: bool
    service_callbacks: int
    input_callbacks: int
    phase_before: bool
    phase_routed: bool
    spans: list[SpanRecord]


class Built(TypedDict):
    logger: OpenTelemetryV2
    exporter: InMemorySpanExporter
    tracer: Tracer


class AuthenticationError(Exception):
    message: str
    status_code: int
    llm_provider: str

    def __init__(self) -> None:
        self.message = "litellm.AuthenticationError: bad credentials"
        self.status_code = 401
        self.llm_provider = "openai"
        super().__init__(self.message)


def run(input_value: Input) -> Answer:
    function = input_value["function"]
    if function == "init":
        return run_init()
    if function == "registration":
        return run_registration()
    if function == "service":
        return run_service(input_value)
    if function == "guardrail":
        return run_guardrail()
    if function == "precall":
        return run_precall(input_value)
    return run_phase()


def build() -> Built:
    config = OpenTelemetryV2Config()
    exporter = InMemorySpanExporter()
    provider = build_tracer_provider(config, exporter=exporter, use_simple_processor=True)
    logger = OpenTelemetryV2(config, tracer_provider=provider)
    return {"logger": logger, "exporter": exporter, "tracer": provider.get_tracer("litellm")}


def answer(spans: list[SpanRecord], phase_before: bool, phase_routed: bool) -> Answer:
    return {
        "constructed": True,
        "service_callbacks": len(litellm.service_callback),
        "input_callbacks": len(litellm.input_callback),
        "phase_before": phase_before,
        "phase_routed": phase_routed,
        "spans": spans,
    }


def records(exporter: InMemorySpanExporter, root_id: int) -> list[SpanRecord]:
    return [record(span, root_id) for span in exporter.get_finished_spans()]


def record(span: ReadableSpan, root_id: int) -> SpanRecord:
    parent: Literal["root", "none", "other"] = "none"
    if span.parent is not None and span.parent.span_id != INVALID_SPAN_ID:
        parent = "root" if span.parent.span_id == root_id else "other"
    return {
        "name": span.name,
        "parent": parent,
        "status": status_label(span.status.status_code),
        "events": len(span.events),
        "error_type": attribute(span, "error.type"),
        "error_message": attribute(span, "error.message"),
        "error_code": attribute(span, "litellm.provider.error.code"),
        "service": attribute(span, "litellm.service.name"),
        "guardrail": attribute(span, "litellm.guardrail.name"),
    }


def status_label(code: StatusCode) -> Literal["unset", "ok", "error"]:
    if code == StatusCode.ERROR:
        return "error"
    if code == StatusCode.OK:
        return "ok"
    return "unset"


def attribute(span: ReadableSpan, key: str) -> str | None:
    value = span.attributes.get(key)
    return None if value is None else str(value)


def run_init() -> Answer:
    build()
    return answer([], False, False)


def run_registration() -> Answer:
    build()
    build()
    return answer([], False, False)


def run_service(input_value: Input) -> Answer:
    built = build()
    root = built["tracer"].start_span("root")
    payload = ServiceLoggerPayload(
        is_error=input_value["error"] is not None,
        service=ServiceTypes.LITELLM if input_value["metrics_only"] else ServiceTypes.REDIS,
        duration=1.5,
        call_type=input_value["call_type"],
        caller="test",
        event_metadata=None,
    )
    error = input_value["error"]
    if error is None:
        asyncio.run(
            built["logger"].async_service_success_hook(
                payload, parent_otel_span=root, start_time=START, end_time=END
            )
        )
    else:
        asyncio.run(
            built["logger"].async_service_failure_hook(
                payload, error, parent_otel_span=root, start_time=START, end_time=END
            )
        )
    return answer(records(built["exporter"], root.get_span_context().span_id), False, False)


def run_guardrail() -> Answer:
    first = build()
    build()
    root = first["tracer"].start_span("root")
    set_request_root_span(root)
    entry: StandardLoggingGuardrailInformation = {
        "guardrail_name": "pii",
        "guardrail_status": "success",
        "start_time": START,
        "end_time": END,
    }
    emit_guardrail_span(entry)
    return answer(records(first["exporter"], root.get_span_context().span_id), False, False)


def run_precall(input_value: Input) -> Answer:
    built = build()
    root = built["tracer"].start_span("root")
    error = AuthenticationError()
    built["logger"].record_error_attributes_on_span(root, error, input_value["status"])
    if input_value["second_status"] > 0:
        built["logger"].record_error_attributes_on_span(root, error, input_value["second_status"])
    root.end()
    return answer(records(built["exporter"], root.get_span_context().span_id), False, False)


def run_phase() -> Answer:
    with phase_span("auth") as before:
        routed_before = before is not None
    built = build()
    with phase_span("auth") as after:
        routed_after = after is not None
    return answer(records(built["exporter"], INVALID_SPAN_ID), routed_before, routed_after)
