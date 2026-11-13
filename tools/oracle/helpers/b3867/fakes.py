import json
from collections.abc import AsyncIterable, Mapping
from typing import TypeVar

from litellm.rust_bridge.traces import ClickHouseStorage, QueryScope
from litellm.tracing import TracingPayloadTooLargeError
from litellm.tracing.decode import InvalidOTLPPayloadError
from litellm.tracing.receiver import Tenant, TraceReceiver, TracingOverloadedError
from litellm.tracing.store import TraceStore
from litellm.tracing.types import (
    SpanDetail,
    SpanErrorPage,
    SpanRow,
    Trace,
    TracePage,
    TraceScope,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .types import DecoderInput, ErrorInput, QueryScript, ReadScript, ScriptInput

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
SPAN_ROWS: TypeAdapter[tuple[SpanRow, ...]] = TypeAdapter(tuple[SpanRow, ...])
PAGE: TypeAdapter[TracePage] = TypeAdapter(TracePage)
TRACE: TypeAdapter[Trace | None] = TypeAdapter(Trace | None)
SPAN: TypeAdapter[SpanDetail | None] = TypeAdapter(SpanDetail | None)
ERROR_PAGE: TypeAdapter[SpanErrorPage | None] = TypeAdapter(SpanErrorPage | None)

ReadValue = TypeVar("ReadValue")

DEFAULT_DECODER_REJECTION = "OTLP payload rejected"
DEFAULT_READ_FAILURE = "the trace read failed"
DEFAULT_QUERY_FAILURE = "the trace query failed"


def json_object(fields: Mapping[str, JsonValue]) -> JsonValue:
    result: JsonValue = dict(fields)
    return result


def scope_json(scope: TraceScope) -> JsonValue:
    return json_object(
        {
            "team_ids": JSON.validate_python(scope["team_ids"]),
            "api_key_hash": scope["api_key_hash"],
        }
    )


def tenant_json(tenant: Tenant) -> JsonValue:
    return json_object(
        {
            "team_id": tenant.team_id,
            "api_key_hash": tenant.api_key_hash,
            "org_id": tenant.org_id,
        }
    )


def decoder_message(script: DecoderInput) -> str:
    message = script.get("message")
    if message is None:
        return DEFAULT_DECODER_REJECTION
    return message


def rows_json(rows: tuple[SpanRow, ...]) -> JsonValue:
    return JSON.validate_json(json.dumps(rows, default=dict).encode())


def ingest_rows(script: DecoderInput, tenant: Tenant) -> JsonValue:
    rows = script.get("rows")
    if rows is None:
        return json_object({"kind": "ingest", "tenant": tenant_json(tenant), "rows": []})
    stamped = tenant.stamp_rows(SPAN_ROWS.validate_python(rows))
    return json_object(
        {
            "kind": "ingest",
            "tenant": tenant_json(tenant),
            "rows": rows_json(stamped),
        }
    )


def read_error(error: ErrorInput) -> ValueError | RuntimeError:
    message = error.get("message")
    if message is None:
        message = DEFAULT_READ_FAILURE
    if error["kind"] == "invalid_cursor":
        return ValueError(message)
    return RuntimeError(message)


def read_answer(section: ReadScript, adapter: TypeAdapter[ReadValue]) -> ReadValue:
    if "error" in section:
        raise read_error(section["error"])
    if "value" not in section:
        raise ValueError("the read script needs a value or an error")
    return adapter.validate_python(section["value"])


def query_answer(section: QueryScript | None) -> str:
    if section is None:
        raise ValueError("the case has no query script")
    if "error" in section:
        error = section["error"]
        message = error.get("message")
        if message is None:
            message = DEFAULT_QUERY_FAILURE
        if error["kind"] == "unavailable":
            raise RuntimeError(message)
        raise ValueError(message)
    if "value" not in section:
        raise ValueError("the query script needs a value or an error")
    return section["value"]


class FakeStorage(ClickHouseStorage):
    def __init__(self, script: ScriptInput) -> None:
        self.script = script
        self.received: JsonValue = None

    @override
    async def query_sql(self, sql: str, scope: QueryScope, secret: str) -> str:
        self.received = json_object(
            {
                "kind": "query",
                "sql": sql,
                "scope": JSON.validate_python(scope),
                "secret": secret,
            }
        )
        return query_answer(self.script.get("query"))

    @override
    async def query_help(self, scope: QueryScope, secret: str) -> str:
        self.received = json_object(
            {
                "kind": "query_help",
                "scope": JSON.validate_python(scope),
                "secret": secret,
            }
        )
        return query_answer(self.script.get("query_help"))


class ScriptedReceiver(TraceReceiver):
    def __init__(self, storage: FakeStorage, script: ScriptInput) -> None:
        self.script = script
        self.storage = storage
        self.store = TraceStore(storage)
        self.received: JsonValue = None

    def decoder(self) -> DecoderInput:
        found = self.script.get("decoder")
        if found is None:
            return DecoderInput(kind="ok", spans=[])
        return found

    @override
    async def ingest(
        self,
        body: bytes | AsyncIterable[bytes],
        content_type: str | None,
        content_encoding: str | None,
        tenant: Tenant,
    ) -> int:
        del body, content_type, content_encoding
        script = self.decoder()
        kind = script["kind"]
        message = decoder_message(script)
        if kind in {"too_large", "overflow"}:
            raise TracingPayloadTooLargeError(message)
        if kind == "malformed":
            raise InvalidOTLPPayloadError(message)
        if kind == "overloaded":
            raise TracingOverloadedError(message)
        self.received = ingest_rows(script, tenant)
        return 1

    @override
    async def list_traces(
        self, scope: TraceScope, start_ms: int, end_ms: int, cursor: str | None = None
    ) -> TracePage:
        self.received = json_object(
            {
                "kind": "list_traces",
                "scope": scope_json(scope),
                "start_ms": start_ms,
                "end_ms": end_ms,
                "cursor": cursor,
            }
        )
        found = self.script.get("list_traces")
        if found is None:
            raise ValueError("the case has no list_traces script")
        return read_answer(found, PAGE)

    @override
    async def get_trace(
        self, trace_id: str, scope: TraceScope, trace_ref: str = ""
    ) -> Trace | None:
        self.received = json_object(
            {
                "kind": "get_trace",
                "trace_id": trace_id,
                "trace_ref": trace_ref,
                "scope": scope_json(scope),
            }
        )
        found = self.script.get("get_trace")
        if found is None:
            raise ValueError("the case has no get_trace script")
        return read_answer(found, TRACE)

    @override
    async def get_span(
        self, trace_id: str, span_id: str, scope: TraceScope, trace_ref: str = ""
    ) -> SpanDetail | None:
        self.received = json_object(
            {
                "kind": "get_span",
                "trace_id": trace_id,
                "span_id": span_id,
                "trace_ref": trace_ref,
                "scope": scope_json(scope),
            }
        )
        found = self.script.get("get_span")
        if found is None:
            raise ValueError("the case has no get_span script")
        return read_answer(found, SPAN)

    @override
    async def get_span_error(
        self,
        trace_id: str,
        span_id: str,
        scope: TraceScope,
        trace_ref: str = "",
        cursor: str | None = None,
    ) -> SpanErrorPage | None:
        self.received = json_object(
            {
                "kind": "get_span_error",
                "trace_id": trace_id,
                "span_id": span_id,
                "trace_ref": trace_ref,
                "cursor": cursor,
                "scope": scope_json(scope),
            }
        )
        found = self.script.get("get_span_error")
        if found is None:
            raise ValueError("the case has no get_span_error script")
        return read_answer(found, ERROR_PAGE)
