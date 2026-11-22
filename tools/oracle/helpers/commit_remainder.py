import asyncio
import sys
from datetime import timedelta
from types import ModuleType, TracebackType
from typing import Final, Literal, Self, TypedDict

import httpx
from litellm.proxy._types import DailyUserSpendTransaction
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.db_spend_update_writer import DBSpendUpdateWriter
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from pydantic import JsonValue
from typing_extensions import override

Token = Literal[
    "ok", "connect_error", "read_timeout", "read_error",
    "data_rejected", "deadlock", "unknown_sqlstate", "other_failure",
]

# asyncio.create_task in the source's reporter needs this many turns to run its failure handler
DRAIN_TICKS: Final = 8


class Call(TypedDict):
    retries: int
    rows: int
    script: list[Token]


class Input(TypedDict):
    calls: list[Call]


class ScriptedDatabaseError(Exception):
    pass


class ScriptedDataError(ScriptedDatabaseError):
    meta: dict[str, str]

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.meta = {"code": code}


class PrismaModule(ModuleType):
    errors: "PrismaModule"
    PrismaError: type[Exception]
    DataError: type[Exception]
    ClientNotConnectedError: type[Exception]


# the source resolves prisma's error classes lazily; the oracle environment has no prisma package
PRISMA: Final = PrismaModule("prisma")
PRISMA.PrismaError = ScriptedDatabaseError
PRISMA.DataError = ScriptedDataError
PRISMA.ClientNotConnectedError = ScriptedDatabaseError
PRISMA.errors = PRISMA
sys.modules.update({"prisma": PRISMA, "prisma.errors": PRISMA})


class Session:
    def __init__(self, script: list[Token]) -> None:
        self.script = script
        self.attempts = 0
        self.reported = 0


class Recorder(ProxyLogging):
    def __init__(self, session: Session) -> None:
        super().__init__(UserApiKeyCache())
        self.session = session

    @override
    async def failure_handler(
        self,
        original_exception: Exception,
        duration: float,
        call_type: str,
        traceback_str: str = "",
    ) -> None:
        del original_exception, duration, call_type, traceback_str
        self.session.reported += 1


class ScriptedTransaction:
    def __init__(self, session: Session) -> None:
        self.session = session

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...

    async def execute_raw(self, _query: str, *_args: JsonValue) -> int:
        self.session.attempts += 1
        step: Token = self.session.script.pop(0) if self.session.script else "ok"
        if step == "connect_error":
            raise httpx.ConnectError(step)
        if step == "read_timeout":
            raise httpx.ReadTimeout(step)
        if step == "read_error":
            raise httpx.ReadError(step)
        if step == "data_rejected":
            raise ScriptedDataError("23514", step)
        if step == "deadlock":
            raise ScriptedDatabaseError("write conflict or a deadlock detected")
        if step == "unknown_sqlstate":
            raise ScriptedDataError("42P01", step)
        if step == "other_failure":
            raise ScriptedDatabaseError(step)
        return 0


class FakeDb(PrismaWrapper):
    def __init__(self, session: Session) -> None:
        super().__init__(object())
        self.session = session

    def tx(self, *, timeout: timedelta) -> ScriptedTransaction:
        del timeout
        return ScriptedTransaction(self.session)


class FakeClient(PrismaClient):
    def __init__(self, session: Session) -> None:
        self.db = FakeDb(session)


class CaseWriter(DBSpendUpdateWriter):
    async def update_daily(
        self,
        *,
        retries: int,
        client: PrismaClient,
        recorder: ProxyLogging,
        transactions: dict[str, DailyUserSpendTransaction],
    ) -> None:
        await self._update_daily_spend(
            n_retry_times=retries, prisma_client=client, proxy_logging_obj=recorder,
            daily_spend_transactions=transactions, entity_type="user", entity_id_field="user_id",
        )


def transaction(index: int) -> DailyUserSpendTransaction:
    return DailyUserSpendTransaction(
        date="2026-01-02",
        user_id=f"user-{index:03d}",
        api_key="key-1",
        model="model-1",
        model_group=None,
        mcp_namespaced_tool_name=None,
        custom_llm_provider="openai",
        endpoint=None,
        prompt_tokens=0, completion_tokens=0, cache_read_input_tokens=0,
        cache_creation_input_tokens=0, compression_saved_tokens=0,
        compression_savings_spend=0.0, prompt_caching_savings_spend=0.0,
        gateway_injected_caching_savings_spend=0.0, spend=1.0,
        api_requests=1, successful_requests=1, failed_requests=0,
    )


async def replay_call(call: Call) -> dict[str, JsonValue]:
    session = Session(list(call["script"]))
    transactions = {
        f"row-{index:03d}": transaction(index) for index in range(call["rows"])
    }
    message = ""
    try:
        await CaseWriter().update_daily(
            retries=call["retries"],
            client=FakeClient(session),
            recorder=Recorder(session),
            transactions=transactions,
        )
    except (httpx.HTTPError, ScriptedDatabaseError) as error:
        message = str(error)
    for _ in range(DRAIN_TICKS):
        await asyncio.sleep(0)
    remaining: list[JsonValue] = []
    remaining.extend(sorted(transactions))
    return {
        "attempts": session.attempts,
        "remaining": remaining,
        "reported": session.reported,
        "error": message,
    }


def run(input_value: Input) -> dict[str, JsonValue]:
    calls: list[JsonValue] = [asyncio.run(replay_call(call)) for call in input_value["calls"]]
    return {"calls": calls}
