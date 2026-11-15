import asyncio
import logging
import os
import sys
from collections.abc import Coroutine
from dataclasses import dataclass
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

from litellm.proxy._experimental.mcp_server.db import (
    rotate_mcp_server_credentials_master_key,
    rotate_mcp_user_credentials_master_key,
    rotate_mcp_user_env_vars_master_key,
)
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

SALT_ENV: Final = "LITELLM_SALT_KEY"
LOGGER_NAME: Final = "LiteLLM Proxy"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
SERVER_TABLE: Final = "litellm_mcpservertable"
OAUTH_TABLE: Final = "litellm_mcpserveroauthclient"
CREDENTIAL_TABLE: Final = "litellm_mcpusercredentials"
ENV_VAR_TABLE: Final = "litellm_mcpuserenvvars"
JSON_TEXT_FIELDS: Final[frozenset[str]] = frozenset({"credentials", "env_vars"})
JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


class InjectedWriteError(Exception):
    pass


class ProxyServerStub(ModuleType):
    master_key: str | None
    general_settings: dict[str, JsonValue]
    redis_usage_cache: None

    def __init__(self) -> None:
        super().__init__(PROXY_SERVER_NAME)
        self.master_key = None
        self.general_settings = {"encryption_algorithm": "aes-256-gcm"}
        self.redis_usage_cache = None


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.INFO)
        self.entries: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            level = "error"
        elif record.levelno >= logging.WARNING:
            level = "warn"
        else:
            level = "info"
        self.entries.append(level)


class FixedRandom:
    def __init__(self, nonce: bytes) -> None:
        self.nonce = nonce

    def __call__(self, size: int) -> bytes:
        return self.nonce[:size]


class StoredRow:
    pass


@dataclass
class ServerRow(StoredRow):
    server_id: str
    credentials: JsonValue
    env_vars: JsonValue
    static_headers: JsonValue
    env: JsonValue


@dataclass
class OAuthRow(StoredRow):
    server_id: str
    credentials: JsonValue


@dataclass
class CredentialRow(StoredRow):
    user_id: str
    server_id: str
    credential_b64: str


@dataclass
class EnvVarRow(StoredRow):
    user_id: str
    server_id: str
    values_b64: str


class Recorder:
    def __init__(self, fail_at: int | None) -> None:
        self.fail_at = fail_at
        self.writes: list[JsonValue] = []

    def add(self, write: JsonValue) -> None:
        if self.fail_at is not None and self.fail_at == len(self.writes):
            raise InjectedWriteError
        self.writes.append(write)


class RecordingTable:
    def __init__(self, table: str, rows: list[StoredRow], recorder: Recorder) -> None:
        self.table = table
        self.rows = rows
        self.recorder = recorder

    async def find_many(self, where: dict[str, JsonValue] | None = None) -> list[StoredRow]:
        del where
        return self.rows

    async def update(self, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        entry: JsonValue = {
            "table": self.table,
            "where": where,
            "data": stored_data(data),
        }
        self.recorder.add(entry)


class FakeDb(PrismaWrapper):
    def __init__(self, tables: dict[str, RecordingTable]) -> None:
        self.tables = tables

    @override
    def __getattr__(self, name: str) -> RecordingTable:
        return self.tables[name]


class FakeClient(PrismaClient):
    def __init__(self, tables: dict[str, RecordingTable]) -> None:
        self.db = FakeDb(tables)


class ServerInput(TypedDict):
    server_id: str
    credentials: NotRequired[JsonValue]
    env_vars: NotRequired[JsonValue]
    static_headers: NotRequired[JsonValue]
    env: NotRequired[JsonValue]


class OAuthInput(TypedDict):
    server_id: str
    credentials: NotRequired[JsonValue]


class CredentialInput(TypedDict):
    user_id: str
    server_id: str
    credential_b64: str


class EnvVarInput(TypedDict):
    user_id: str
    server_id: str
    values_b64: str


class RotateServers(TypedDict):
    function: Literal["rotate_servers"]
    salt_key: str
    new_master_key: str
    nonce_hex: str
    touched_by: str
    servers: list[ServerInput]
    oauth_clients: list[OAuthInput]
    fail_at_write: NotRequired[int]


class RotateUserCredentials(TypedDict):
    function: Literal["rotate_user_credentials"]
    salt_key: str
    new_master_key: str
    nonce_hex: str
    rows: list[CredentialInput]
    fail_at_write: NotRequired[int]


class RotateUserEnvVars(TypedDict):
    function: Literal["rotate_user_env_vars"]
    salt_key: str
    new_master_key: str
    nonce_hex: str
    rows: list[EnvVarInput]
    fail_at_write: NotRequired[int]


Input = RotateServers | RotateUserCredentials | RotateUserEnvVars


def stored_data(data: dict[str, JsonValue]) -> dict[str, JsonValue]:
    stored: dict[str, JsonValue] = {}
    for name, value in data.items():
        if name in JSON_TEXT_FIELDS:
            stored[name] = JSON_VALUE.validate_json(TEXT.validate_python(value))
        else:
            stored[name] = value
    return stored


def server_rows(servers: list[ServerInput]) -> list[StoredRow]:
    return [
        ServerRow(
            server_id=server["server_id"],
            credentials=server.get("credentials"),
            env_vars=server.get("env_vars"),
            static_headers=server.get("static_headers"),
            env=server.get("env"),
        )
        for server in servers
    ]


def oauth_rows(clients: list[OAuthInput]) -> list[StoredRow]:
    return [
        OAuthRow(
            server_id=client["server_id"],
            credentials=client.get("credentials"),
        )
        for client in clients
    ]


def credential_rows(rows: list[CredentialInput]) -> list[StoredRow]:
    return [
        CredentialRow(
            user_id=row["user_id"],
            server_id=row["server_id"],
            credential_b64=row["credential_b64"],
        )
        for row in rows
    ]


def env_var_rows(rows: list[EnvVarInput]) -> list[StoredRow]:
    return [
        EnvVarRow(
            user_id=row["user_id"],
            server_id=row["server_id"],
            values_b64=row["values_b64"],
        )
        for row in rows
    ]


def run_case(
    value: Input,
    recorder: Recorder,
    rotation: Coroutine[None, None, None],
) -> JsonValue:
    os.environ[SALT_ENV] = value["salt_key"]
    sys.modules[PROXY_SERVER_NAME] = ProxyServerStub()
    capture = LogCapture()
    logger = logging.getLogger(LOGGER_NAME)
    saved_level = logger.level
    saved_propagate = logger.propagate
    saved_handlers = list(logger.handlers)
    for existing in saved_handlers:
        logger.removeHandler(existing)
    logger.setLevel(logging.INFO)
    logger.propagate = False
    logger.addHandler(capture)
    fixed = FixedRandom(bytes.fromhex(value["nonce_hex"]))
    saved_random = os.urandom
    os.urandom = fixed
    failed = False
    try:
        asyncio.run(rotation)
    except InjectedWriteError:
        failed = True
    finally:
        os.urandom = saved_random
        logger.removeHandler(capture)
        for existing in saved_handlers:
            logger.addHandler(existing)
        logger.setLevel(saved_level)
        logger.propagate = saved_propagate
    return {"failed": failed, "writes": recorder.writes, "logs": capture.entries}


def rotate_servers(value: RotateServers) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    tables = {
        SERVER_TABLE: RecordingTable("servers", server_rows(value["servers"]), recorder),
        OAUTH_TABLE: RecordingTable("oauth_clients", oauth_rows(value["oauth_clients"]), recorder),
        CREDENTIAL_TABLE: RecordingTable("user_credentials", [], recorder),
        ENV_VAR_TABLE: RecordingTable("user_env_vars", [], recorder),
    }
    rotation: Coroutine[None, None, None] = rotate_mcp_server_credentials_master_key(
        FakeClient(tables), value["touched_by"], value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def rotate_user_credentials(value: RotateUserCredentials) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    tables = {
        SERVER_TABLE: RecordingTable("servers", [], recorder),
        OAUTH_TABLE: RecordingTable("oauth_clients", [], recorder),
        CREDENTIAL_TABLE: RecordingTable("user_credentials", credential_rows(value["rows"]), recorder),
        ENV_VAR_TABLE: RecordingTable("user_env_vars", [], recorder),
    }
    rotation: Coroutine[None, None, None] = rotate_mcp_user_credentials_master_key(
        FakeClient(tables), value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def rotate_user_env_vars(value: RotateUserEnvVars) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    tables = {
        SERVER_TABLE: RecordingTable("servers", [], recorder),
        OAUTH_TABLE: RecordingTable("oauth_clients", [], recorder),
        CREDENTIAL_TABLE: RecordingTable("user_credentials", [], recorder),
        ENV_VAR_TABLE: RecordingTable("user_env_vars", env_var_rows(value["rows"]), recorder),
    }
    rotation: Coroutine[None, None, None] = rotate_mcp_user_env_vars_master_key(
        FakeClient(tables), value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "rotate_servers":
            answer = rotate_servers(input_value)
        case "rotate_user_credentials":
            answer = rotate_user_credentials(input_value)
        case "rotate_user_env_vars":
            answer = rotate_user_env_vars(input_value)
    return answer
