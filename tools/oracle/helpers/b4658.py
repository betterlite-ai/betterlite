from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.bridge_token_flow import (
    _active_key_user_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/bridge_token_flow.py#L152-L156
    _active_user_record,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/bridge_token_flow.py#L350-L353
    _key_is_active,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/bridge_token_flow.py#L115-L149
    _litellm_key_from_request,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/bridge_token_flow.py#L35-L54
)
from litellm.proxy._types import LiteLLM_UserTable, UserAPIKeyAuth
from pydantic import JsonValue
from starlette.requests import Request
from starlette.types import Scope


class KeyFields(TypedDict, total=False):
    token: str
    user_id: str
    team_id: str
    blocked: bool
    expires: str


class UserFields(TypedDict):
    user_id: str
    metadata: NotRequired[dict[str, JsonValue] | None]


class PresentedKeyCall(TypedDict):
    operation: Literal["presented_key"]
    headers: list[tuple[str, str]]


class KeyIsActiveCall(TypedDict):
    operation: Literal["key_is_active"]
    key: KeyFields


class ActiveKeyUserIdCall(TypedDict):
    operation: Literal["active_key_user_id"]
    key: KeyFields


class UserRecordCall(TypedDict):
    operation: Literal["user_record"]
    user: UserFields


Input: TypeAlias = PresentedKeyCall | KeyIsActiveCall | ActiveKeyUserIdCall | UserRecordCall


def request_of(headers: list[tuple[str, str]]) -> Request:
    # an ASGI server lowercases header names before Starlette reads them
    raw: list[tuple[bytes, bytes]] = [
        (name.lower().encode("latin-1"), value.encode("latin-1"))
        for name, value in headers
    ]
    scope: Scope = {"type": "http", "headers": raw}
    return Request(scope=scope)


def auth_of(fields: KeyFields) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        token=fields.get("token"),
        user_id=fields.get("user_id"),
        team_id=fields.get("team_id"),
        blocked=fields.get("blocked"),
        expires=fields.get("expires"),
    )


def user_of(fields: UserFields) -> LiteLLM_UserTable:
    return LiteLLM_UserTable(user_id=fields["user_id"], metadata=fields.get("metadata"))


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "presented_key":
            return _litellm_key_from_request(request_of(input_value["headers"]))
        case "key_is_active":
            return _key_is_active(auth_of(input_value["key"]))
        case "active_key_user_id":
            return _active_key_user_id(auth_of(input_value["key"]))
        case "user_record":
            record = _active_user_record(user_of(input_value["user"]))
            return "no_active_key" if isinstance(record, str) else "active"
