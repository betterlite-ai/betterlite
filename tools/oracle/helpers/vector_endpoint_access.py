from typing import Final, Literal, TypedDict

from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.vector_store_endpoints.utils import (
    check_vector_store_permission,
    is_allowed_to_call_vector_store_endpoint,
    is_allowed_to_call_vector_store_files_endpoint,
)
from litellm.types.utils import LlmProviders
from pydantic import JsonValue
from starlette.requests import Request


class PermissionInput(TypedDict):
    function: Literal["permission"]
    index_name: str
    permission: Literal["read", "write"]
    key_metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None


class EndpointInput(TypedDict):
    function: Literal["store", "files"]
    provider: str
    id: str
    method: str
    path: str
    app_root_path: str | None
    root_path: str | None
    user_role: str | None
    key_metadata: dict[str, JsonValue] | None
    team_metadata: dict[str, JsonValue] | None


Input = PermissionInput | EndpointInput

EMPTY_METADATA: Final[dict[str, JsonValue]] = {}


def permission(fields: PermissionInput) -> JsonValue:
    return check_vector_store_permission(
        index_name=fields["index_name"],
        permission=fields["permission"],
        key_metadata=fields["key_metadata"],
        team_metadata=fields["team_metadata"],
    )


def caller_for(fields: EndpointInput) -> UserAPIKeyAuth:
    role = fields["user_role"]
    metadata = fields["key_metadata"]
    return UserAPIKeyAuth(
        metadata=metadata if metadata is not None else EMPTY_METADATA,
        team_metadata=fields["team_metadata"],
        user_role=LitellmUserRoles(role) if role is not None else None,
    )


def build_request(fields: EndpointInput) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    scope: dict[str, str | bytes | list[tuple[bytes, bytes]]] = {
        "type": "http",
        "method": fields["method"],
        "path": fields["path"],
        "headers": headers,
        "query_string": b"",
    }
    app_root_path = fields["app_root_path"]
    if app_root_path is not None:
        scope["app_root_path"] = app_root_path
    root_path = fields["root_path"]
    if root_path is not None:
        scope["root_path"] = root_path
    return Request(scope=scope)


def endpoint(fields: EndpointInput, function: Literal["store", "files"]) -> JsonValue:
    caller = caller_for(fields)
    request = build_request(fields)
    provider = LlmProviders(fields["provider"])
    if function == "store":
        return is_allowed_to_call_vector_store_endpoint(provider, fields["id"], request, caller)
    return is_allowed_to_call_vector_store_files_endpoint(provider, fields["id"], request, caller)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "permission":
        return permission(input_value)
    if input_value["function"] == "store":
        return endpoint(input_value, "store")
    return endpoint(input_value, "files")
