import asyncio
import logging
import os
import sys
from collections.abc import Mapping
from types import ModuleType
from typing import Final, Literal

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOAssertionCache,
)
from litellm.proxy.common_utils.encrypt_decrypt_utils import decrypt_value_helper
from pydantic import JsonValue
from typing_extensions import override

from b10010.prisma_double import (
    AssertionRow,
    AssertionTable,
    FakePrismaClient,
    ServerRow,
    ServerTable,
)
from b10011.model import (
    Answer,
    Failure,
    Input,
    LogEntry,
    StoredEntry,
    render_payload,
)
from b10011.steps import run_step

SALT_ENV: Final = "LITELLM_SALT_KEY"
PROXY_LOGGER: Final = "LiteLLM Proxy"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
MANAGER_NAME: Final = "litellm.proxy._experimental.mcp_server.mcp_server_manager"
ASSERTION_FIELD: Final = "sso_identity_assertion"
WARNING_PREFIX: Final = "Failed to persist the SSO identity assertion for EMA egress"
# the Rust side of this operation encrypts under the same key
SALT_KEY: Final = "b10011-salt-key"
FAKE_FAILURE: Final = "the in-memory assertion rows failed"


def fails_now(failure: Failure | None, method: Literal["upsert", "find_first"], calls: int) -> bool:
    if failure is None:
        return False
    return failure["method"] == method and failure["call"] == calls


class CountingAssertions(AssertionTable):
    def __init__(self, failure: Failure | None) -> None:
        super().__init__([])
        self.upserts = 0
        self.failure = failure

    @override
    async def upsert(
        self,
        *,
        where: Mapping[str, str],
        data: Mapping[str, Mapping[str, str]],
    ) -> None:
        if fails_now(self.failure, "upsert", self.upserts):
            raise RuntimeError(FAKE_FAILURE)
        await super().upsert(where=where, data=data)
        self.upserts += 1


class CountingServers(ServerTable):
    def __init__(self, rows: list[ServerRow], failure: Failure | None) -> None:
        super().__init__(rows)
        self.reads = 0
        self.failure = failure

    @override
    async def find_first(self, *, where: Mapping[str, str]) -> ServerRow | None:
        if fails_now(self.failure, "find_first", self.reads):
            raise RuntimeError(FAKE_FAILURE)
        found = await super().find_first(where=where)
        self.reads += 1
        return found


class ConfigServer:
    def __init__(self, auth_type: str | None) -> None:
        self.auth_type = auth_type


class ManagerStub:
    def __init__(self, servers: dict[str, ConfigServer]) -> None:
        self.config_mcp_servers: dict[str, ConfigServer] = servers


class ManagerModule(ModuleType):
    def __init__(self, servers: dict[str, ConfigServer]) -> None:
        super().__init__(MANAGER_NAME)
        self.global_mcp_server_manager: ManagerStub = ManagerStub(servers)


class ProxyServerStub(ModuleType):
    def __init__(self) -> None:
        super().__init__(PROXY_SERVER_NAME)
        self.master_key: str | None = None
        self.general_settings: dict[str, JsonValue] = {}
        self.prisma_client: FakePrismaClient | None = None


class WarningLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.entries: list[LogEntry] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(WARNING_PREFIX):
            return
        self.entries.append(LogEntry(level=record.levelname, message=message))


def capture_warnings(capture: WarningLog) -> logging.Logger:
    logger = logging.getLogger(PROXY_LOGGER)
    # the proxy logger's handlers write to stdout, the channel the helper answer travels on
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(capture)
    return logger


def stored_assertion(row: AssertionRow) -> JsonValue:
    plaintext = decrypt_value_helper(row.assertion_b64, ASSERTION_FIELD, exception_type="debug")
    if plaintext is None:
        return None
    return render_payload(plaintext)


def stored_entries(assertions: CountingAssertions) -> list[StoredEntry]:
    return [
        StoredEntry(user_id=row.user_id, assertion=stored_assertion(row))
        for row in assertions.rows
    ]


async def answer(input_value: Input, proxy: ProxyServerStub, capture: WarningLog) -> Answer:
    assertions = CountingAssertions(input_value["failure"])
    servers = CountingServers(
        [ServerRow(f"server-{index}", auth_type) for index, auth_type in enumerate(input_value["db_servers"])],
        input_value["failure"],
    )
    if input_value["database"]:
        proxy.prisma_client = FakePrismaClient(assertions, servers)
    cache = SSOAssertionCache()
    steps = [await run_step(step, cache) for step in input_value["steps"]]
    return Answer(
        steps=steps,
        stored=stored_entries(assertions),
        reads=servers.reads,
        writes=assertions.upserts,
        logs=capture.entries,
    )


def run(input_value: Input) -> Answer:
    os.environ[SALT_ENV] = SALT_KEY
    proxy = ProxyServerStub()
    sys.modules[PROXY_SERVER_NAME] = proxy
    declared = {
        f"server-{index}": ConfigServer(auth_type)
        for index, auth_type in enumerate(input_value["config_servers"])
    }
    sys.modules[MANAGER_NAME] = ManagerModule(declared)
    capture = WarningLog()
    logger = capture_warnings(capture)
    try:
        return asyncio.run(answer(input_value, proxy, capture))
    finally:
        logger.removeHandler(capture)
