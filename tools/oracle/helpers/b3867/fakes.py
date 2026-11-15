import json
from collections.abc import AsyncIterable
from typing import NotRequired, TypeVar, TypedDict

from litellm.rust_bridge.traces import ClickHouseStorage, QueryScope
from litellm.tracing import TracingPayloadTooLargeError
from litellm.tracing.decode import InvalidOTLPPayloadError
from litellm.tracing.receiver import Tenant, TraceReceiver, TracingOverloadedError
from litellm.tracing.store import TraceStore
from litellm.tracing.types import (
    SpanDetail,
    SpanErrorPage,
    Trace,
    TracePage,
    TraceScope,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
PAGE: TypeAdapter[TracePage] = TypeAdapter(TracePage)
TRACE: TypeAdapter[Trace | None] = TypeAdapter(Trace | None)
SPAN: TypeAdapter[SpanDetail | None] = TypeAdapter(SpanDetail | None)
ERROR_PAGE: TypeAdapter[SpanErrorPage | None] = TypeAdapter(SpanErrorPage | None)

DEFAULT_MESSAGE = "the trace read failed"
DEFAULT_REJECTION = "OTLP payload rejected"


class RequestInput(TypedDict):
    method: str
    path: str
    headers: NotRequired[dict[str, str]]
    query: NotRequired[dict[str, str]]
    body: NotRequired[str]
    body_base64: NotRequired[bool]


class AuthInput(TypedDict):
    user_role: NotRequired[str]
    team_id: NotRequired[str]
    token: NotRequired[str]
    project_id: NotRequired[str]
    org_id: NotRequired[str]


class Script(TypedDict):
    decoder: NotRequired[JsonValue]
    list_traces: NotRequired[JsonValue]
    get_trace: NotRequired[JsonValue]
    get_span: NotRequired[JsonValue]
    get_span_error: NotRequired[JsonValue]


class Input(TypedDict):
    request: RequestInput
    auth: NotRequired[AuthInput]
    tracing: NotRequired[bool]
    master_key: NotRequired[str]
    script: NotRequired[Script]


ReadValue = TypeVar("ReadValue")


def read_answer(value: JsonValue, adapter: TypeAdapter[ReadValue]) -> ReadValue:
    payload = OBJECT.validate_python(value)
    if "value" in payload:
        return adapter.validate_python(payload["value"])
    error = payload.get("error")
    if error is None:
        raise ValueError(DEFAULT_MESSAGE)
    failure = OBJECT.validate_python(error)
    message = failure.get("message")
    text = DEFAULT_MESSAGE if message is None else str(message)
    if failure.get("kind") == "invalid_cursor":
        raise ValueError(text)
    raise RuntimeError(text)


def read_script(
    value: JsonValue | None, message: str, adapter: TypeAdapter[ReadValue]
) -> ReadValue:
    if value is None:
        raise ValueError(message)
    return read_answer(value, adapter)


def decoder_state(script: Script) -> tuple[str, str]:
    section = script.get("decoder")
    payload = OBJECT.validate_python(section) if section is not None else {}
    kind = payload.get("kind")
    message = payload.get("message")
    text = DEFAULT_REJECTION if message is None else str(message)
    return ("ok" if kind is None else str(kind), text)


def echo(scope: QueryScope, secret: str) -> str:
    payload: dict[str, JsonValue] = {
        "scope": OBJECT.validate_python(scope),
        "secret": secret,
    }
    return json.dumps(payload, separators=(",", ":"), sort_keys=True)


class CannedStorage(ClickHouseStorage):
    def __init__(self) -> None:
        # the base constructor opens the native bridge, which the oracle environment lacks
        pass

    @override
    async def query_sql(self, sql: str, scope: QueryScope, secret: str) -> str:
        del sql
        return echo(scope, secret)

    @override
    async def query_help(self, scope: QueryScope, secret: str) -> str:
        return echo(scope, secret)


class ScriptedReceiver(TraceReceiver):
    def __init__(self, script: Script) -> None:
        self.script = script
        self.store = TraceStore(CannedStorage())

    @override
    async def ingest(
        self,
        body: bytes | AsyncIterable[bytes],
        content_type: str | None,
        content_encoding: str | None,
        tenant: Tenant,
    ) -> int:
        del body, content_type, content_encoding, tenant
        kind, message = decoder_state(self.script)
        if kind in {"too_large", "overflow"}:
            raise TracingPayloadTooLargeError(message)
        if kind == "malformed":
            raise InvalidOTLPPayloadError(message)
        if kind == "overloaded":
            raise TracingOverloadedError(message)
        return 1

    @override
    async def list_traces(
        self, scope: TraceScope, start_ms: int, end_ms: int, cursor: str | None = None
    ) -> TracePage:
        del scope, start_ms, end_ms, cursor
        return read_script(self.script.get("list_traces"), "no list_traces script", PAGE)

    @override
    async def get_trace(
        self, trace_id: str, scope: TraceScope, trace_ref: str = ""
    ) -> Trace | None:
        del trace_id, scope, trace_ref
        return read_script(self.script.get("get_trace"), "no get_trace script", TRACE)

    @override
    async def get_span(
        self, trace_id: str, span_id: str, scope: TraceScope, trace_ref: str = ""
    ) -> SpanDetail | None:
        del trace_id, span_id, scope, trace_ref
        return read_script(self.script.get("get_span"), "no get_span script", SPAN)

    @override
    async def get_span_error(
        self,
        trace_id: str,
        span_id: str,
        scope: TraceScope,
        trace_ref: str = "",
        cursor: str | None = None,
    ) -> SpanErrorPage | None:
        del trace_id, span_id, scope, trace_ref, cursor
        return read_script(
            self.script.get("get_span_error"), "no get_span_error script", ERROR_PAGE
        )
