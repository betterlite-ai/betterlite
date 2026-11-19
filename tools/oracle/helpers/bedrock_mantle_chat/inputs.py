from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue

Object: TypeAlias = dict[str, JsonValue]


class UrlCall(TypedDict):
    model: str
    api_base: str | None
    optional_params: Object
    headers: dict[str, str]
    metadata: dict[str, str]
    project_id: str | None
    policy_fields: list[str] | None
    environment: dict[str, str]


class UrlCases(TypedDict):
    function: Literal["chat_url", "message_url"]
    calls: list[UrlCall]


class MessagesCall(TypedDict):
    function: Literal["messages_request"]
    model_cost: Object
    model: str
    messages: list[JsonValue]
    optional_params: Object
    request_headers: dict[str, str]


class ChatCall(TypedDict):
    function: Literal["chat_request", "remote_url_inlined"]
    model_cost: Object
    model: str
    messages: list[JsonValue]
    optional_params: Object
    litellm_params: Object
    headers: dict[str, str]
    fetch: dict[str, str]


class NativeSseCall(TypedDict):
    function: Literal["native_sse"]
    events: list[str]


class ResponseCall(TypedDict):
    function: Literal["messages_response"]
    body: str
    status: int


Input: TypeAlias = UrlCases | MessagesCall | ChatCall | NativeSseCall | ResponseCall


class EnvironmentArgs(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[JsonValue]
    optional_params: Object
    litellm_params: Object
    api_key: str | None
    api_base: str | None


class UrlArgs(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    optional_params: Object
    litellm_params: Object
    stream: bool | None
