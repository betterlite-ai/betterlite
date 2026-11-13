from typing import Literal, NotRequired, TypedDict

from litellm.rust_bridge.traces import DecodedSpan
from litellm.tracing.types import SpanRow
from pydantic import JsonValue


class BodyInput(TypedDict):
    encoding: Literal["utf8", "base64"]
    value: str
    hanging: NotRequired[bool]


class RequestInput(TypedDict):
    method: str
    path: str
    headers: NotRequired[dict[str, str]]
    body: NotRequired[BodyInput]
    query: NotRequired[dict[str, str]]


class AuthInput(TypedDict):
    user_role: NotRequired[str]
    team_id: NotRequired[str]
    token: NotRequired[str]
    project_id: NotRequired[str]
    org_id: NotRequired[str]


class DecoderInput(TypedDict):
    kind: Literal["ok", "overflow", "malformed", "too_large", "overloaded"]
    message: NotRequired[str]
    spans: NotRequired[list[DecodedSpan]]
    rows: NotRequired[list[SpanRow]]


class ErrorInput(TypedDict):
    kind: Literal["invalid_cursor", "invalid", "unavailable", "storage"]
    message: NotRequired[str]


class ReadScript(TypedDict):
    value: NotRequired[JsonValue]
    error: NotRequired[ErrorInput]


class QueryScript(TypedDict):
    value: NotRequired[str]
    error: NotRequired[ErrorInput]


class ScriptInput(TypedDict):
    decoder: NotRequired[DecoderInput]
    max_concurrent_ingests: NotRequired[int]
    body_read_timeout_ms: NotRequired[int]
    now_ms: NotRequired[int]
    list_traces: NotRequired[ReadScript]
    get_trace: NotRequired[ReadScript]
    get_span: NotRequired[ReadScript]
    get_span_error: NotRequired[ReadScript]
    query: NotRequired[QueryScript]
    query_help: NotRequired[QueryScript]


class Input(TypedDict):
    request: RequestInput
    auth: NotRequired[AuthInput]
    tracing: NotRequired[bool]
    master_key: NotRequired[str]
    script: NotRequired[ScriptInput]
