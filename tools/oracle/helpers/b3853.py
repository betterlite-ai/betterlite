import asyncio
from typing import Literal, TypedDict

import litellm
from litellm.integrations.otel.logger import OpenTelemetryV2, emit_guardrail_span, phase_span
from litellm.integrations.otel.model.config import OpenTelemetryV2Config
from litellm.integrations.otel.plumbing.context import set_request_root_span
from litellm.integrations.otel.plumbing.providers import build_tracer_provider
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from opentelemetry.sdk.trace import ReadableSpan
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import INVALID_SPAN_ID, Span, StatusCode

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
    kind: str
    start: int | None
    end: int | None


class Answer(TypedDict):
    service_callbacks: int
    input_callbacks: int
    phase_before: bool
    phase_routed: bool
    spans: list[SpanRecord]


class AuthenticationError(Exception):
    message: str = "litellm.AuthenticationError: bad credentials"
    status_code: int = 401
    llm_provider: str = "openai"


def run(input_value: Input) -> Answer:
    function = input_value["function"]
    if function in ("init", "registration"):
        _, exporter, _ = build()
        if function == "registration":
            build()
        return answer(exporter, INVALID_SPAN_ID, phase_before=False, phase_routed=False)
    if function == "service":
        return run_service(input_value)
    if function == "guardrail":
        return run_guardrail()
    if function == "precall":
        return run_precall(input_value)
    return run_phase()


def build() -> tuple[OpenTelemetryV2, InMemorySpanExporter, Span]:
    config = OpenTelemetryV2Config()
    exporter = InMemorySpanExporter()
    provider = build_tracer_provider(config, exporter=exporter, use_simple_processor=True)
    logger = OpenTelemetryV2(config, tracer_provider=provider)
    return logger, exporter, provider.get_tracer("litellm").start_span("root")


def answer(
    exporter: InMemorySpanExporter, root_id: int, *, phase_before: bool, phase_routed: bool
) -> Answer:
    return {
        "service_callbacks": len(litellm.service_callback),  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L118
        "input_callbacks": len(litellm.input_callback),  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L115
        "phase_before": phase_before,
        "phase_routed": phase_routed,
        "spans": [record(span, root_id) for span in exporter.get_finished_spans()],
    }


def record(span: ReadableSpan, root_id: int) -> SpanRecord:
    parent: Literal["root", "none", "other"] = "none"
    if span.parent is not None and span.parent.span_id != INVALID_SPAN_ID:
        parent = "root" if span.parent.span_id == root_id else "other"
    code = span.status.status_code
    status: Literal["unset", "ok", "error"] = (
        "error" if code == StatusCode.ERROR else "ok" if code == StatusCode.OK else "unset"
    )
    return {
        "name": span.name,
        "parent": parent,
        "status": status,
        "events": len(span.events),
        "kind": span.kind.name.lower(),
        "start": span.start_time if parent == "root" else None,
        "end": span.end_time if parent == "root" else None,
    }


def run_service(input_value: Input) -> Answer:
    logger, exporter, root = build()
    error = input_value["error"]
    payload = ServiceLoggerPayload(
        is_error=error is not None,
        error=None,
        service=ServiceTypes.LITELLM if input_value["metrics_only"] else ServiceTypes.REDIS,
        duration=1.5,
        call_type=input_value["call_type"],
        caller="test",
        event_metadata=None,
    )
    if error is None:
        coroutine = logger.async_service_success_hook(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L603
            payload, parent_otel_span=root, start_time=START, end_time=END
        )
    else:
        coroutine = logger.async_service_failure_hook(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L620
            payload, error, parent_otel_span=root, start_time=START, end_time=END
        )
    asyncio.run(coroutine)
    return answer(exporter, root.get_span_context().span_id, phase_before=False, phase_routed=False)


def run_guardrail() -> Answer:
    _, exporter, root = build()
    build()
    set_request_root_span(root)
    emit_guardrail_span(
        {
            "guardrail_name": "pii",
            "guardrail_status": "success",
            "start_time": START,
            "end_time": END,
        }
    )
    return answer(exporter, root.get_span_context().span_id, phase_before=False, phase_routed=False)


def run_precall(input_value: Input) -> Answer:
    logger, exporter, root = build()
    error = AuthenticationError()
    logger.record_error_attributes_on_span(root, error, input_value["status"])
    second = input_value["second_status"]
    if second > 0:
        logger.record_error_attributes_on_span(root, error, second)
    root.end()
    return answer(exporter, root.get_span_context().span_id, phase_before=False, phase_routed=False)


def run_phase() -> Answer:
    with phase_span("auth") as before:
        routed_before = before is not None
    _, exporter, _ = build()
    with phase_span("auth") as after:
        routed_after = after is not None
    return answer(exporter, INVALID_SPAN_ID, phase_before=routed_before, phase_routed=routed_after)
