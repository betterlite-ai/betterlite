import asyncio
import logging
import os
import sys
from types import ModuleType
from typing import Final

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    DbSSOAssertionStore,
    SSOAssertionCache,
)
from pydantic import JsonValue
from typing_extensions import override

from b10010.prisma_double import AssertionRow, FakePrismaClient, ServerTable
from b10064.model import Answer, Input, LogEntry
from b10064.steps import run_step
from b10064.table import CountingAssertions

SALT_ENV: Final = "LITELLM_SALT_KEY"
PROXY_LOGGER: Final = "LiteLLM Proxy"
PROXY_SERVER_NAME: Final = "litellm.proxy.proxy_server"
WARNING_PREFIX: Final = "Stored SSO identity assertion for user_id="
SALT_KEY: Final = "b10064-salt-key"


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
    for handler in list(logger.handlers):
        logger.removeHandler(handler)
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(capture)
    return logger


async def answer(input_value: Input, proxy: ProxyServerStub, capture: WarningLog) -> Answer:
    cache = SSOAssertionCache()
    failure = input_value["failure"]
    failure_call = None if failure is None else failure["call"]
    assertions = CountingAssertions(
        [AssertionRow(row["user_id"], row["assertion_b64"]) for row in input_value["rows"]],
        failure_call,
        cache,
    )
    if input_value["database"]:
        proxy.prisma_client = FakePrismaClient(assertions, ServerTable([]))
    store = DbSSOAssertionStore(cache=cache)
    steps = [await run_step(step, store, cache, assertions) for step in input_value["steps"]]
    return Answer(steps=steps, logs=capture.entries)


def run(input_value: Input) -> Answer:
    os.environ[SALT_ENV] = SALT_KEY
    proxy = ProxyServerStub()
    sys.modules[PROXY_SERVER_NAME] = proxy
    capture = WarningLog()
    logger = capture_warnings(capture)
    try:
        return asyncio.run(answer(input_value, proxy, capture))
    finally:
        logger.removeHandler(capture)
