from typing import Literal, NotRequired, TypedDict

from pydantic import JsonValue


class BodyInput(TypedDict):
    encoding: Literal["utf8", "base64"]
    value: str


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


class Script(TypedDict):
    decoder: NotRequired[JsonValue]
    list_traces: NotRequired[JsonValue]
    get_trace: NotRequired[JsonValue]
    get_span: NotRequired[JsonValue]
    get_span_error: NotRequired[JsonValue]
    query: NotRequired[JsonValue]
    query_help: NotRequired[JsonValue]


class Input(TypedDict):
    request: RequestInput
    auth: NotRequired[AuthInput]
    tracing: NotRequired[bool]
    master_key: NotRequired[str]
    script: NotRequired[Script]
