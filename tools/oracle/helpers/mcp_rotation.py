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
    def __init__(self) -> None:
        super().__init__(PROXY_SERVER_NAME)
        self.master_key: str | None = None
        self.general_settings: dict[str, JsonValue] = {"encryption_algorithm": "aes-256-gcm"}
        self.redis_usage_cache: None = None


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.INFO)
        self.entries: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            self.entries.append("error")
        elif record.levelno >= logging.WARNING:
            self.entries.append("warn")
        else:
            self.entries.append("info")


class FixedRandom:
    def __init__(self, nonce: bytes) -> None:
        self.nonce = nonce

    def __call__(self, size: int) -> bytes:
        return self.nonce[:size]


@dataclass
class Row:
    server_id: str
    credentials: JsonValue = None
    env_vars: JsonValue = None
    static_headers: JsonValue = None
    env: JsonValue = None
    user_id: str = ""
    credential_b64: str = ""
    values_b64: str = ""


class Recorder:
    def __init__(self, fail_at: int | None) -> None:
        self.fail_at = fail_at
        self.writes: list[JsonValue] = []

    def add(self, write: JsonValue) -> None:
        if self.fail_at is not None and self.fail_at == len(self.writes):
            raise InjectedWriteError
        self.writes.append(write)


class RecordingTable:
    def __init__(self, table: str, rows: list[Row], recorder: Recorder) -> None:
        self.table = table
        self.rows = rows
        self.recorder = recorder

    async def find_many(self, where: dict[str, JsonValue] | None = None) -> list[Row]:
        del where
        return self.rows

    async def update(self, where: dict[str, JsonValue], data: dict[str, JsonValue]) -> None:
        stored: dict[str, JsonValue] = {
            name: JSON_VALUE.validate_json(TEXT.validate_python(value))
            if name in JSON_TEXT_FIELDS
            else value
            for name, value in data.items()
        }
        self.recorder.add({"table": self.table, "where": where, "data": stored})


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


class CaseInput(TypedDict):
    salt_key: str
    new_master_key: str
    nonce_hex: str
    fail_at_write: NotRequired[int]


class RotateServers(CaseInput):
    function: Literal["rotate_servers"]
    touched_by: str
    servers: list[ServerInput]
    oauth_clients: list[OAuthInput]


class RotateUserCredentials(CaseInput):
    function: Literal["rotate_user_credentials"]
    rows: list[CredentialInput]


class RotateUserEnvVars(CaseInput):
    function: Literal["rotate_user_env_vars"]
    rows: list[EnvVarInput]


Input = RotateServers | RotateUserCredentials | RotateUserEnvVars


def run_case(value: Input, recorder: Recorder, rotation: Coroutine[None, None, None]) -> JsonValue:
    os.environ[SALT_ENV] = value["salt_key"]
    sys.modules[PROXY_SERVER_NAME] = ProxyServerStub()
    capture = LogCapture()
    logger = logging.getLogger(LOGGER_NAME)
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
    logger.setLevel(logging.INFO)
    logger.propagate = False
    logger.addHandler(capture)
    os.urandom = FixedRandom(bytes.fromhex(value["nonce_hex"]))
    failed = False
    try:
        asyncio.run(rotation)
    except InjectedWriteError:
        failed = True
    return {"failed": failed, "writes": recorder.writes, "logs": capture.entries}


def rotate_servers(value: RotateServers) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    oauth_clients = [
        Row(client["server_id"], client.get("credentials")) for client in value["oauth_clients"]
    ]
    servers = [
        Row(
            server["server_id"],
            server.get("credentials"),
            server.get("env_vars"),
            server.get("static_headers"),
            server.get("env"),
        )
        for server in value["servers"]
    ]
    tables = {
        SERVER_TABLE: RecordingTable("servers", servers, recorder),
        OAUTH_TABLE: RecordingTable("oauth_clients", oauth_clients, recorder),
    }
    rotation: Coroutine[None, None, None] = rotate_mcp_server_credentials_master_key(
        FakeClient(tables), value["touched_by"], value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def rotate_user_credentials(value: RotateUserCredentials) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    rows = [
        Row(row["server_id"], user_id=row["user_id"], credential_b64=row["credential_b64"])
        for row in value["rows"]
    ]
    table = RecordingTable("user_credentials", rows, recorder)
    rotation: Coroutine[None, None, None] = rotate_mcp_user_credentials_master_key(
        FakeClient({CREDENTIAL_TABLE: table}), value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def rotate_user_env_vars(value: RotateUserEnvVars) -> JsonValue:
    recorder = Recorder(value.get("fail_at_write"))
    rows = [
        Row(row["server_id"], user_id=row["user_id"], values_b64=row["values_b64"])
        for row in value["rows"]
    ]
    table = RecordingTable("user_env_vars", rows, recorder)
    rotation: Coroutine[None, None, None] = rotate_mcp_user_env_vars_master_key(
        FakeClient({ENV_VAR_TABLE: table}), value["new_master_key"]
    )
    return run_case(value, recorder, rotation)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "rotate_servers":
        return rotate_servers(input_value)
    if input_value["function"] == "rotate_user_credentials":
        return rotate_user_credentials(input_value)
    return rotate_user_env_vars(input_value)
