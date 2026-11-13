import asyncio
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

from .fakes import ERROR_PAGE, PAGE, SPAN, TRACE, FakeStorage, ScriptedReceiver, json_object
from .types import AuthInput, BodyInput, Input, RequestInput, ScriptInput

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

ROLES: Final[dict[str, LitellmUserRoles]] = {
    "proxy_admin": LitellmUserRoles.PROXY_ADMIN,
    "proxy_admin_view_only": LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY,
}


class BodyChannel:
    def __init__(self, body: bytes, *, hanging: bool) -> None:
        self.body = body
        self.hanging = hanging

    async def __call__(self) -> Message:
        if self.hanging:
            await asyncio.Event().wait()
        message: Message = {"type": "http.request", "body": self.body, "more_body": False}
        return message


def body_bytes(fields: BodyInput) -> bytes:
    if fields["encoding"] == "base64":
        return base64.b64decode(fields["value"])
    return fields["value"].encode()


def build_request(fields: RequestInput) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    configured = fields.get("headers")
    if configured is not None:
        for name, value in configured.items():
            headers.append((name.encode("latin-1"), value.encode("latin-1")))
    query = fields.get("query")
    query_string = urlencode(query).encode() if query is not None else b""
    scope: Scope = {
        "type": "http",
        "method": fields["method"],
        "path": fields["path"],
        "query_string": query_string,
        "headers": headers,
    }
    body = fields.get("body")
    if body is None:
        channel = BodyChannel(b"", hanging=False)
    else:
        channel = BodyChannel(body_bytes(body), hanging=body.get("hanging") is True)
    return Request(scope, channel)


def build_auth(fields: AuthInput | None) -> UserAPIKeyAuth:
    if fields is None:
        return UserAPIKeyAuth(user_role=LitellmUserRoles.INTERNAL_USER)
    configured = fields.get("user_role")
    role = None if configured is None else ROLES.get(configured)
    if role is None:
        role = LitellmUserRoles.INTERNAL_USER
    return UserAPIKeyAuth(
        user_role=role,
        team_id=fields.get("team_id"),
        token=fields.get("token"),
        project_id=fields.get("project_id"),
        org_id=fields.get("org_id"),
    )


def build_receiver(
    script: ScriptInput, *, enabled: bool
) -> tuple[FakeStorage, ScriptedReceiver, TraceReceiver | None]:
    storage = FakeStorage(script)
    receiver = ScriptedReceiver(storage, script)
    if not enabled:
        return storage, receiver, None
    return storage, receiver, receiver


def parsed_body(body: bytes) -> JsonValue:
    try:
        return JSON.validate_json(body)
    except ValidationError:
        return None


SINGLE_TRACE_SEGMENTS = 4
SPAN_SEGMENTS = 6
SPAN_ERROR_SEGMENTS = 7


def answer_value(
    status: int,
    content_type: str | None,
    body: bytes,
    retry_after: str | None,
    received: JsonValue,
) -> JsonValue:
    return json_object(
        {
            "status": status,
            "content_type": content_type,
            "body": parsed_body(body),
            "body_base64": base64.b64encode(body).decode(),
            "retry_after": retry_after,
            "received": received,
        }
    )


def response_value(response: Response, received: JsonValue) -> JsonValue:
    return answer_value(
        response.status_code,
        response.headers.get("content-type"),
        bytes(response.body),
        response.headers.get("retry-after"),
        received,
    )


def query_window(fields: RequestInput) -> tuple[int | None, int | None, str | None]:
    query = fields.get("query")
    if query is None:
        return None, None, None
    start = query.get("start_ms")
    end = query.get("end_ms")
    start_ms = None if start is None else int(start)
    end_ms = None if end is None else int(end)
    return start_ms, end_ms, query.get("cursor")


def trace_ref(fields: RequestInput) -> str:
    query = fields.get("query")
    if query is None:
        return ""
    found = query.get("trace_ref")
    if found is None:
        return ""
    return found


async def read_route(
    fields: RequestInput,
    auth: UserAPIKeyAuth,
    receiver: TraceReceiver | None,
    received: JsonValue,
) -> JsonValue:
    parts = fields["path"].split("/")
    context = await provide_trace_access(auth, receiver)
    reference = trace_ref(fields)
    if len(parts) == SINGLE_TRACE_SEGMENTS:
        trace = await get_agent_trace(parts[3], context, reference)
        body = TRACE.dump_json(trace)
        return answer_value(200, "application/json", body, None, received)
    if len(parts) == SPAN_SEGMENTS:
        span = await get_agent_trace_span(parts[3], parts[5], context, reference)
        body = SPAN.dump_json(span)
        return answer_value(200, "application/json", body, None, received)
    if len(parts) == SPAN_ERROR_SEGMENTS and parts[6] == "error":
        query = fields.get("query")
        cursor = None if query is None else query.get("cursor")
        page = await get_agent_trace_span_error(parts[3], parts[5], context, reference, cursor)
        body = ERROR_PAGE.dump_json(page)
        return answer_value(200, "application/json", body, None, received)
    raise ValueError(f"the b3867 helper does not route {fields['method']} {fields['path']}")


async def dispatch(input_value: Input) -> JsonValue:
    fields = input_value["request"]
    script = input_value.get("script")
    if script is None:
        script = ScriptInput()
    enabled = input_value.get("tracing")
    if enabled is None:
        enabled = True
    auth = build_auth(input_value.get("auth"))
    storage, receiver, receiver_option = build_receiver(script, enabled=enabled)
    proxy_server.master_key = input_value.get("master_key")
    request = build_request(fields)
    method = fields["method"]
    path = fields["path"]
    if path == "/v1/traces" and method == "POST":
        context = await provide_trace_access(auth, receiver_option)
        response = await ingest_otlp_traces(request, context)
        return response_value(response, receiver.received)
    if path == "/v1/traces" and method == "GET":
        start_ms, end_ms, cursor = query_window(fields)
        page = await list_agent_traces(
            await provide_trace_access(auth, receiver_option),
            start_ms,
            end_ms,
            cursor,
        )
        body = PAGE.dump_json(page)
        return answer_value(200, "application/json", body, None, receiver.received)
    if path == "/v1/traces/query" and method == "POST":
        secret = provide_trace_query_secret()
        access = await provide_trace_query_access(auth, receiver_option, secret)
        body_fields = fields.get("body")
        raw = b"" if body_fields is None else body_bytes(body_fields)
        response = await query_agent_traces(TraceQueryRequest.model_validate_json(raw), access)
        return response_value(response, storage.received)
    if path == "/v1/traces/query/help" and method == "GET":
        secret = provide_trace_query_secret()
        access = await provide_trace_query_access(auth, receiver_option, secret)
        response = await help_agent_trace_queries(access)
        return response_value(response, storage.received)
    if path.startswith("/v1/traces/") and method == "GET":
        return await read_route(fields, auth, receiver_option, receiver.received)
    raise ValueError(f"the b3867 helper does not route {method} {path}")
