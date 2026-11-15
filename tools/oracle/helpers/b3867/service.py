import base64
from typing import Final
from urllib.parse import urlencode

from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.tracing_endpoints import (
    TraceQueryRequest,
    get_agent_trace,
    get_agent_trace_span,
    get_agent_trace_span_error,
    help_agent_trace_queries,
    ingest_otlp_traces,
    list_agent_traces,
    provide_trace_access,
    provide_trace_query_access,
    provide_trace_query_secret,
    query_agent_traces,
)
from litellm.tracing.receiver import TraceReceiver
from pydantic import JsonValue, TypeAdapter, ValidationError
from starlette.requests import Request
from starlette.responses import Response
from starlette.types import Message, Scope

from .fakes import ScriptedReceiver
from .types import ERROR_PAGE, PAGE, SPAN, TRACE, AuthInput, Input, RequestInput, Script

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

ROLES: Final[dict[str, LitellmUserRoles]] = {
    "proxy_admin": LitellmUserRoles.PROXY_ADMIN,
    "proxy_admin_view_only": LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY,
}

SINGLE_TRACE = 4
SPAN_SEGMENTS = 6
SPAN_ERROR_SEGMENTS = 7

INGEST_PATH = "/v1/traces"
QUERY_PATH = "/v1/traces/query"
HELP_PATH = "/v1/traces/query/help"


class BodyChannel:
    def __init__(self, body: bytes) -> None:
        self.body = body

    async def __call__(self) -> Message:
        message: Message = {"type": "http.request", "body": self.body, "more_body": False}
        return message


def ingest_request(fields: RequestInput) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    configured = fields.get("headers")
    if configured is not None:
        for name, value in configured.items():
            headers.append((name.encode("latin-1"), value.encode("latin-1")))
    query = fields.get("query")
    scope: Scope = {
        "type": "http",
        "method": fields["method"],
        "path": fields["path"],
        "query_string": b"" if query is None else urlencode(query).encode(),
        "headers": headers,
    }
    body = fields.get("body")
    raw = b"" if body is None else body_bytes(fields, body)
    return Request(scope, BodyChannel(raw))


def body_bytes(fields: RequestInput, body: str) -> bytes:
    if fields.get("body_base64") is True:
        return base64.b64decode(body)
    return body.encode()


def build_auth(fields: AuthInput | None) -> UserAPIKeyAuth:
    if fields is None:
        return UserAPIKeyAuth(user_role=LitellmUserRoles.INTERNAL_USER)
    configured = fields.get("user_role")
    role = None if configured is None else ROLES.get(configured)
    return UserAPIKeyAuth(
        user_role=LitellmUserRoles.INTERNAL_USER if role is None else role,
        team_id=fields.get("team_id"),
        token=fields.get("token"),
        project_id=fields.get("project_id"),
        org_id=fields.get("org_id"),
    )


def parsed_body(body: bytes) -> JsonValue:
    try:
        return JSON.validate_json(body)
    except ValidationError:
        return None


def reply(status: int, content_type: str | None, body: bytes, retry_after: str | None) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "status": status,
        "content_type": content_type,
        "body": parsed_body(body),
        "body_base64": base64.b64encode(body).decode(),
        "retry_after": retry_after,
    }
    return payload


def response_value(response: Response) -> JsonValue:
    return reply(
        response.status_code,
        response.headers.get("content-type"),
        bytes(response.body),
        response.headers.get("retry-after"),
    )


def window(fields: RequestInput) -> tuple[int | None, int | None, str | None]:
    query = fields.get("query")
    if query is None:
        return None, None, None
    start = query.get("start_ms")
    end = query.get("end_ms")
    return (
        None if start is None else int(start),
        None if end is None else int(end),
        query.get("cursor"),
    )


def trace_ref(fields: RequestInput) -> str:
    query = fields.get("query")
    if query is None:
        return ""
    found = query.get("trace_ref")
    return "" if found is None else found


async def read_route(
    fields: RequestInput, auth: UserAPIKeyAuth, receiver: TraceReceiver | None
) -> JsonValue:
    parts = fields["path"].split("/")
    context = await provide_trace_access(auth, receiver)
    reference = trace_ref(fields)
    if len(parts) == SINGLE_TRACE:
        trace = await get_agent_trace(parts[3], context, reference)
        return reply(200, "application/json", TRACE.dump_json(trace), None)
    if len(parts) == SPAN_SEGMENTS:
        span = await get_agent_trace_span(parts[3], parts[5], context, reference)
        return reply(200, "application/json", SPAN.dump_json(span), None)
    if len(parts) == SPAN_ERROR_SEGMENTS and parts[6] == "error":
        query = fields.get("query")
        cursor = None if query is None else query.get("cursor")
        page = await get_agent_trace_span_error(parts[3], parts[5], context, reference, cursor)
        return reply(200, "application/json", ERROR_PAGE.dump_json(page), None)
    raise ValueError(f"the b3867 helper does not route {fields['method']} {fields['path']}")


async def dispatch(input_value: Input) -> JsonValue:
    fields = input_value["request"]
    configured = input_value.get("script")
    script = Script() if configured is None else configured
    auth = build_auth(input_value.get("auth"))
    enabled = input_value.get("tracing")
    receiver: TraceReceiver | None = None
    if enabled is None or enabled:
        receiver = ScriptedReceiver(script)
    proxy_server.master_key = input_value.get("master_key")
    method = fields["method"]
    path = fields["path"]
    if method == "POST" and path == INGEST_PATH:
        context = await provide_trace_access(auth, receiver)
        return response_value(await ingest_otlp_traces(ingest_request(fields), context))
    if method == "GET" and path == INGEST_PATH:
        start_ms, end_ms, cursor = window(fields)
        page = await list_agent_traces(
            await provide_trace_access(auth, receiver), start_ms, end_ms, cursor
        )
        return reply(200, "application/json", PAGE.dump_json(page), None)
    if method == "POST" and path == QUERY_PATH:
        access = await provide_trace_query_access(
            auth, receiver, provide_trace_query_secret()
        )
        body = fields.get("body")
        raw = b"" if body is None else body_bytes(fields, body)
        response = await query_agent_traces(TraceQueryRequest.model_validate_json(raw), access)
        return response_value(response)
    if method == "GET" and path == HELP_PATH:
        access = await provide_trace_query_access(
            auth, receiver, provide_trace_query_secret()
        )
        return response_value(await help_agent_trace_queries(access))
    if method == "GET":
        return await read_route(fields, auth, receiver)
    raise ValueError(f"the b3867 helper does not route {method} {path}")
