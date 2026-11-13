import asyncio
import os
from types import TracebackType
from typing import Final, Literal, NotRequired, Self, TypedDict

from litellm.proxy._experimental.mcp_server.db import (
    create_mcp_server,
    decrypt_credentials,
    decrypt_global_env_var_values,
    encrypt_credentials,
    update_mcp_server,
)
from litellm.proxy._types import MCPEnvVar, NewMCPServerRequest, UpdateMCPServerRequest
from litellm.proxy.common_utils.encrypt_decrypt_utils import decode_secret_map
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.mcp import MCPCredentials
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

AUDIT_FIELDS: Final[frozenset[str]] = frozenset({"created_by", "updated_by"})
JSON_TEXT_FIELDS: Final[frozenset[str]] = frozenset({
    "credentials",
    "env",
    "env_vars",
    "mcp_info",
    "static_headers",
    "tool_name_to_description",
    "tool_name_to_display_name",
})


class CapturedWriteError(Exception):
    pass


class RecordingTable:
    def __init__(self) -> None:
        self.writes: list[dict[str, JsonValue]] = []
        self.reads = 0

    async def create(self, data: dict[str, JsonValue]) -> None:
        self.writes.append(data)
        raise CapturedWriteError

    async def update(self, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        del where
        self.writes.append(data)
        raise CapturedWriteError

    async def find_unique(self, where: dict[str, JsonValue]) -> None:
        self.reads += 1
        del where

    async def find_first(self, where: dict[str, JsonValue]) -> None:
        self.reads += 1
        del where

    def __call__(self) -> "RecordingTransaction":
        return RecordingTransaction(self)


class RecordingTransaction:
    def __init__(self, table: RecordingTable) -> None:
        self.table = table

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        del exc_type, exc, traceback

    async def execute_raw(self, query: str, *args: int) -> int:
        del query, args
        return 0

    @property
    def litellm_mcpservertable(self) -> RecordingTable:
        return self.table


class FakeDb(PrismaWrapper):
    def __init__(self, table: RecordingTable) -> None:
        self.table = table

    @override
    def __getattr__(self, name: str) -> RecordingTable:
        del name
        return self.table


class FakeClient(PrismaClient):
    def __init__(self, table: RecordingTable) -> None:
        self.db = FakeDb(table)


class FixedRandom:
    def __init__(self, nonce: bytes) -> None:
        self.nonce = nonce

    def __call__(self, size: int) -> bytes:
        return self.nonce[:size]


class EnvVarView(TypedDict, total=False):
    name: str
    value: str
    scope: str
    description: str | None


class RequestFields(TypedDict):
    server_id: NotRequired[str | None]
    server_name: NotRequired[str | None]
    alias: NotRequired[str | None]
    description: NotRequired[str | None]
    transport: NotRequired[Literal["sse", "http", "stdio"]]
    auth_type: NotRequired[str | None]
    credentials: NotRequired[dict[str, JsonValue] | None]
    url: NotRequired[str | None]
    spec_path: NotRequired[str | None]
    mcp_info: NotRequired[dict[str, JsonValue] | None]
    mcp_access_groups: NotRequired[list[str]]
    allowed_tools: NotRequired[list[str] | None]
    tool_name_to_display_name: NotRequired[dict[str, str] | None]
    tool_name_to_description: NotRequired[dict[str, str] | None]
    extra_headers: NotRequired[list[str] | None]
    static_headers: NotRequired[dict[str, str] | None]
    env_vars: NotRequired[list[dict[str, JsonValue]] | None]
    instructions: NotRequired[str | None]
    command: NotRequired[str | None]
    args: NotRequired[list[str]]
    env: NotRequired[dict[str, str]]
    issuer: NotRequired[str | None]
    authorization_url: NotRequired[str | None]
    token_url: NotRequired[str | None]
    registration_url: NotRequired[str | None]
    oauth2_flow: NotRequired[Literal["client_credentials", "authorization_code"] | None]
    token_exchange_endpoint: NotRequired[str | None]
    audience: NotRequired[str | None]
    subject_token_type: NotRequired[str | None]
    token_exchange_profile: NotRequired[str | None]
    allow_all_keys: NotRequired[bool]
    available_on_public_internet: NotRequired[bool]
    delegate_auth_to_upstream: NotRequired[bool]
    oauth_passthrough: NotRequired[bool]
    dcr_bridge: NotRequired[bool | None]
    per_server_oauth_discovery: NotRequired[bool]
    is_byok: NotRequired[bool]
    byok_description: NotRequired[list[str]]
    byok_api_key_help_url: NotRequired[str | None]
    source_url: NotRequired[str | None]
    timeout: NotRequired[float | None]
    max_concurrent_requests: NotRequired[int | None]
    approval_status: NotRequired[str | None]
    submitted_by: NotRequired[str | None]


class PrepareCreate(TypedDict):
    function: Literal["prepare_create"]
    salt_key: str
    request: RequestFields


class PrepareUpdate(TypedDict):
    function: Literal["prepare_update"]
    salt_key: str
    request: RequestFields
    model_fields: list[str]
    original_fields: list[str]


class EncryptCredentials(TypedDict):
    function: Literal["encrypt_credentials"]
    salt_key: str
    credentials: dict[str, JsonValue]
    nonce_hex: str


class DecryptCredentials(TypedDict):
    function: Literal["decrypt_credentials"]
    salt_key: str
    credentials: dict[str, JsonValue]


class DecryptEnvVars(TypedDict):
    function: Literal["decrypt_env_vars"]
    salt_key: str
    env_vars: list[dict[str, JsonValue]]


Input = PrepareCreate | PrepareUpdate | EncryptCredentials | DecryptCredentials | DecryptEnvVars


def env_var_views(env_vars: list[MCPEnvVar]) -> list[EnvVarView]:
    return [
        TypeAdapter(EnvVarView).validate_json(entry.model_dump_json(exclude_unset=True))
        for entry in env_vars
    ]


def plaintext_value(field: str, text: str) -> JsonValue:
    match field:
        case "credentials":
            credentials = TypeAdapter(MCPCredentials).validate_json(text)
            return TypeAdapter(JsonValue).validate_python(decrypt_credentials(credentials))
        case "static_headers" | "env":
            return TypeAdapter(JsonValue).validate_python(decode_secret_map(text, key=field))
        case "env_vars":
            env_vars = TypeAdapter(list[MCPEnvVar]).validate_json(text)
            decrypt_global_env_var_values(env_vars)
            return TypeAdapter(JsonValue).validate_python(env_var_views(env_vars))
        case "mcp_info" | "tool_name_to_description" | "tool_name_to_display_name":
            return TypeAdapter(JsonValue).validate_json(text)
        case other:
            raise ValueError(f"the {other} column is not stored as JSON text")


def plaintext_payload(payload: dict[str, JsonValue]) -> dict[str, JsonValue]:
    view: dict[str, JsonValue] = dict(payload)
    for field in JSON_TEXT_FIELDS:
        stored = payload.get(field)
        if stored is None:
            continue
        view[field] = plaintext_value(field, TypeAdapter(str).validate_python(stored))
    return view


def captured_payload(table: RecordingTable) -> dict[str, JsonValue]:
    payload = plaintext_payload(table.writes[0])
    return {field: entry for field, entry in payload.items() if field not in AUDIT_FIELDS}


def prepare_create(value: PrepareCreate) -> JsonValue:
    table = RecordingTable()
    request = TypeAdapter(NewMCPServerRequest).validate_python(value["request"])
    try:
        asyncio.run(create_mcp_server(FakeClient(table), request, "tester"))
    except CapturedWriteError:
        return TypeAdapter(JsonValue).validate_python(captured_payload(table))
    raise RuntimeError("the create entry returned without writing the prepared payload")


def prepare_update(value: PrepareUpdate) -> JsonValue:
    table = RecordingTable()
    request = TypeAdapter(UpdateMCPServerRequest).validate_python(value["request"])
    original = set(value["original_fields"])
    try:
        asyncio.run(update_mcp_server(FakeClient(table), request, "tester", fields_set=original))
    except CapturedWriteError:
        return TypeAdapter(JsonValue).validate_python(captured_payload(table))
    raise RuntimeError("the update entry returned without writing the prepared payload")


def encrypt(value: EncryptCredentials) -> JsonValue:
    credentials = TypeAdapter(MCPCredentials).validate_python(value["credentials"])
    fixed = FixedRandom(bytes.fromhex(value["nonce_hex"]))
    saved = os.urandom
    os.urandom = fixed
    try:
        encrypted = encrypt_credentials(credentials, None)
    finally:
        os.urandom = saved
    return TypeAdapter(JsonValue).validate_python(encrypted)


def decrypt(value: DecryptCredentials) -> JsonValue:
    credentials = TypeAdapter(MCPCredentials).validate_python(value["credentials"])
    return TypeAdapter(JsonValue).validate_python(decrypt_credentials(credentials))


def decrypt_env(value: DecryptEnvVars) -> JsonValue:
    env_vars = TypeAdapter(list[MCPEnvVar]).validate_python(value["env_vars"])
    decrypt_global_env_var_values(env_vars)
    return TypeAdapter(JsonValue).validate_python(env_var_views(env_vars))


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "prepare_create":
            return prepare_create(input_value)
        case "prepare_update":
            return prepare_update(input_value)
        case "encrypt_credentials":
            return encrypt(input_value)
        case "decrypt_credentials":
            return decrypt(input_value)
        case "decrypt_env_vars":
            return decrypt_env(input_value)
