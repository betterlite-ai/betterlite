from typing import NotRequired, TypedDict

from litellm.tracing.types import SpanDetail, SpanErrorPage, Trace, TracePage
from pydantic import JsonValue, TypeAdapter

PAGE: TypeAdapter[TracePage] = TypeAdapter(TracePage)
TRACE: TypeAdapter[Trace | None] = TypeAdapter(Trace | None)
SPAN: TypeAdapter[SpanDetail | None] = TypeAdapter(SpanDetail | None)
ERROR_PAGE: TypeAdapter[SpanErrorPage | None] = TypeAdapter(SpanErrorPage | None)


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
