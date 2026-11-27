from collections.abc import Iterator
from contextlib import contextmanager
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy._experimental.mcp_server.mcp_server_manager import (
    global_mcp_server_manager,
)
from litellm.proxy._types import LiteLLM_MCPServerTable, NewMCPServerRequest, UserAPIKeyAuth
from litellm.proxy.management_endpoints.mcp_management_endpoints import (
    _has_non_admin_config_credentials,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L874-L882
    _inherit_credentials_from_existing_server,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L884-L917
    _redact_mcp_credentials_list,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L704-L707
    _resolve_credential_target_user_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L718-L739
    _sanitize_mcp_server_for_non_admin,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L752-L793
    _sanitize_mcp_server_for_virtual_key,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L800-L851
    _sanitize_mcp_server_list_for_non_admin,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L795-L798
    _sanitize_mcp_server_list_for_virtual_key,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L853-L856
    _user_is_full_admin,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/mcp_management_endpoints.py#L709-L716
)
from litellm.types.mcp import MCPCredentials
from pydantic import JsonValue, TypeAdapter

from b10079 import servers_by_id


class RegistryCall(TypedDict):
    config_servers: list[dict[str, JsonValue]]
    db_servers: list[dict[str, JsonValue]]
    public_mcp_servers: list[str] | None
    public_hub_strict: bool


class RedactListCall(TypedDict):
    function: Literal["redact_list"]
    registry: RegistryCall
    servers: list[dict[str, JsonValue]]


class NonAdminCall(TypedDict):
    function: Literal["non_admin"]
    registry: RegistryCall
    server: dict[str, JsonValue]


class NonAdminListCall(TypedDict):
    function: Literal["non_admin_list"]
    registry: RegistryCall
    servers: list[dict[str, JsonValue]]


class VirtualKeyCall(TypedDict):
    function: Literal["virtual_key"]
    registry: RegistryCall
    server: dict[str, JsonValue]


class VirtualKeyListCall(TypedDict):
    function: Literal["virtual_key_list"]
    registry: RegistryCall
    servers: list[dict[str, JsonValue]]


class FullAdminCall(TypedDict):
    function: Literal["full_admin"]
    auth: dict[str, JsonValue]


class CredentialTargetCall(TypedDict):
    function: Literal["credential_target"]
    auth: dict[str, JsonValue]
    requested_user_id: str | None


class HasCredentialsCall(TypedDict):
    function: Literal["has_non_admin_credentials"]
    credentials: NotRequired[MCPCredentials | None]


class InheritCall(TypedDict):
    function: Literal["inherit_credentials"]
    registry: RegistryCall
    payload: dict[str, JsonValue]


CredentialCall: TypeAlias = (
    FullAdminCall | CredentialTargetCall | HasCredentialsCall | InheritCall
)
Input: TypeAlias = (
    RedactListCall
    | NonAdminCall
    | NonAdminListCall
    | VirtualKeyCall
    | VirtualKeyListCall
    | CredentialCall
)
Answer: TypeAlias = list[dict[str, JsonValue]] | dict[str, JsonValue] | None


class InheritAnswer(TypedDict):
    credentials: dict[str, JsonValue] | None


SERVER: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
INHERIT_ANSWER: Final[TypeAdapter[InheritAnswer]] = TypeAdapter(InheritAnswer)


@contextmanager
def servers_swapped(call: RegistryCall) -> Iterator[None]:
    saved = (
        global_mcp_server_manager.config_mcp_servers,
        global_mcp_server_manager.registry,
        litellm.public_mcp_servers,
        litellm.public_mcp_hub_strict_whitelist,
    )
    try:
        global_mcp_server_manager.config_mcp_servers = servers_by_id(call["config_servers"])
        global_mcp_server_manager.registry = servers_by_id(call["db_servers"])
        litellm.public_mcp_servers = call["public_mcp_servers"]
        litellm.public_mcp_hub_strict_whitelist = call["public_hub_strict"]
        yield
    finally:
        (
            global_mcp_server_manager.config_mcp_servers,
            global_mcp_server_manager.registry,
            litellm.public_mcp_servers,
            litellm.public_mcp_hub_strict_whitelist,
        ) = saved


def redact_list(call: RedactListCall) -> list[dict[str, JsonValue]]:
    with servers_swapped(call["registry"]):
        servers = [LiteLLM_MCPServerTable.model_validate(item) for item in call["servers"]]
        redacted = _redact_mcp_credentials_list(servers)
        return [SERVER.validate_json(server.model_dump_json()) for server in redacted]


def non_admin(call: NonAdminCall) -> dict[str, JsonValue]:
    with servers_swapped(call["registry"]):
        server = LiteLLM_MCPServerTable.model_validate(call["server"])
        sanitized = _sanitize_mcp_server_for_non_admin(server)
        return SERVER.validate_json(sanitized.model_dump_json())


def non_admin_list(call: NonAdminListCall) -> list[dict[str, JsonValue]]:
    with servers_swapped(call["registry"]):
        servers = [LiteLLM_MCPServerTable.model_validate(item) for item in call["servers"]]
        sanitized = _sanitize_mcp_server_list_for_non_admin(servers)
        return [SERVER.validate_json(server.model_dump_json()) for server in sanitized]


def virtual_key(call: VirtualKeyCall) -> dict[str, JsonValue]:
    with servers_swapped(call["registry"]):
        server = LiteLLM_MCPServerTable.model_validate(call["server"])
        sanitized = _sanitize_mcp_server_for_virtual_key(server)
        return SERVER.validate_json(sanitized.model_dump_json())


def virtual_key_list(call: VirtualKeyListCall) -> list[dict[str, JsonValue]]:
    with servers_swapped(call["registry"]):
        servers = [LiteLLM_MCPServerTable.model_validate(item) for item in call["servers"]]
        sanitized = _sanitize_mcp_server_list_for_virtual_key(servers)
        return [SERVER.validate_json(server.model_dump_json()) for server in sanitized]


def full_admin(call: FullAdminCall) -> dict[str, JsonValue]:
    auth = UserAPIKeyAuth.model_validate(call["auth"])
    return {"full_admin": _user_is_full_admin(auth)}


def credential_target(call: CredentialTargetCall) -> dict[str, JsonValue]:
    auth = UserAPIKeyAuth.model_validate(call["auth"])
    try:
        target = _resolve_credential_target_user_id(auth, call["requested_user_id"])
    except HTTPException as error:
        return {"status": error.status_code}
    return {"user_id": target}


def has_credentials(call: HasCredentialsCall) -> dict[str, JsonValue]:
    supplied = call.get("credentials")
    return {"has_non_admin_config_credentials": _has_non_admin_config_credentials(supplied)}


def inherit_credentials(call: InheritCall) -> dict[str, JsonValue] | None:
    with servers_swapped(call["registry"]):
        payload = NewMCPServerRequest.model_validate(call["payload"])
        inherited = _inherit_credentials_from_existing_server(payload)
        return INHERIT_ANSWER.validate_json(inherited.model_dump_json())["credentials"]


def credential_answer(call: CredentialCall) -> Answer:
    match call["function"]:
        case "full_admin":
            return full_admin(call)
        case "credential_target":
            return credential_target(call)
        case "has_non_admin_credentials":
            return has_credentials(call)
        case "inherit_credentials":
            return inherit_credentials(call)


def run(input_value: Input) -> Answer:
    if input_value["function"] == "redact_list":
        return redact_list(input_value)
    if input_value["function"] == "non_admin":
        return non_admin(input_value)
    if input_value["function"] == "non_admin_list":
        return non_admin_list(input_value)
    if input_value["function"] == "virtual_key":
        return virtual_key(input_value)
    if input_value["function"] == "virtual_key_list":
        return virtual_key_list(input_value)
    return credential_answer(input_value)
