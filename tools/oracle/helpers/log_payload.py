from collections.abc import Awaitable, Callable
from typing import Final, Literal, TypeAlias, TypedDict
from urllib.parse import urlencode

import litellm
from fastapi import FastAPI
from fastapi.testclient import TestClient
from litellm.integrations.additional_logging_utils import AdditionalLoggingUtils
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.logging_callback_manager import LoggingCallbackManager
from litellm.proxy import proxy_server
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.auth.user_api_key_auth import user_api_key_auth
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.spend_tracking.spend_management_endpoints import router
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

AuthCheck: TypeAlias = Callable[..., Awaitable[UserAPIKeyAuth]]
AuthAnswer: TypeAlias = Callable[[], UserAPIKeyAuth]
Bind: TypeAlias = str | int | None
COLD_LOGGER: Final = "s3_v2"


class DetailRow(TypedDict):
    request_id: str
    messages: JsonValue
    response: JsonValue
    proxy_server_request: JsonValue
    metadata: JsonValue


class ColdAnswer(TypedDict):
    payload: dict[str, JsonValue] | None
    fails: bool


class DetailStep(TypedDict):
    operation: Literal["detail"]
    row: DetailRow
    cold: ColdAnswer


class MetadataStep(TypedDict):
    operation: Literal["metadata"]
    values: list[JsonValue]


class SummaryRow(TypedDict):
    day: str
    api_key: str | None
    user: str | None
    model: str
    spend: float


class SummaryStep(TypedDict):
    operation: Literal["daily_summary"]
    start_date: str
    end_date: str
    api_key: str | None
    request_id: str | None
    user_id: str | None
    rows: list[SummaryRow]


Step: TypeAlias = DetailStep | MetadataStep | SummaryStep


class Input(TypedDict):
    steps: list[Step]


class SessionPage(TypedDict):
    data: list[dict[str, JsonValue]]


BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
PAGE: Final[TypeAdapter[SessionPage]] = TypeAdapter(SessionPage)


class ScriptedFailureError(RuntimeError):
    pass


# the spend log table the session route counts before it reads
class SpendTable:
    def __init__(self, total: int) -> None:
        self.total = total

    async def count(self, *, where: dict[str, str]) -> int:
        del where
        return self.total


# every raw read answers the step's rows and is recorded with its binds
class Db(PrismaWrapper):
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = rows
        self.queries: list[tuple[str, list[JsonValue]]] = []
        self.litellm_spendlogs = SpendTable(len(rows))

    async def query_raw(self, sql: str, *args: Bind) -> list[dict[str, JsonValue]]:
        binds: list[JsonValue] = [*args]
        self.queries.append((sql, binds))
        return self.rows


class Client(PrismaClient):
    def __init__(self, db: Db) -> None:
        self.db = db


# the logger litellm.cold_storage_custom_logger names, recording each object key it is asked for
class ColdLogger(CustomLogger):
    def __init__(self, answer: ColdAnswer) -> None:
        super().__init__()
        self.answer = answer
        self.keys: list[str] = []

    @override
    async def get_proxy_server_request_from_cold_storage_with_object_key(
        self, object_key: str
    ) -> dict[str, JsonValue] | None:
        self.keys.append(object_key)
        if self.answer["fails"]:
            raise ScriptedFailureError(object_key)
        return self.answer["payload"]


# no callback serves whole payloads, so the detail route falls through to the row and cold storage
class Loggers(LoggingCallbackManager):
    def __init__(self, cold: ColdLogger) -> None:
        super().__init__()
        self.cold = cold

    @override
    def get_active_additional_logging_utils_from_custom_logger(  # ast-grep-ignore: no-long-names
        self,
    ) -> set[AdditionalLoggingUtils]:
        return set()

    @override
    def get_active_custom_logger_for_callback_name(  # ast-grep-ignore: no-long-names
        self, callback_name: str
    ) -> CustomLogger | None:
        del callback_name
        return self.cold


# the spend routes with an admin in place of the key check
class App(FastAPI):
    dependency_overrides: dict[AuthCheck, AuthAnswer]

    def __init__(self) -> None:
        super().__init__()
        self.caller = UserAPIKeyAuth(user_id="root", user_role=LitellmUserRoles.PROXY_ADMIN)
        self.dependency_overrides = {user_api_key_auth: self.admin}
        self.include_router(router)

    def admin(self) -> UserAPIKeyAuth:
        return self.caller


def run(input_value: Input) -> list[JsonValue]:
    client = proxy_server.prisma_client
    loggers = litellm.logging_callback_manager
    cold_logger = litellm.cold_storage_custom_logger
    try:
        return [answer(step) for step in input_value["steps"]]
    finally:
        proxy_server.prisma_client = client
        litellm.logging_callback_manager = loggers
        litellm.cold_storage_custom_logger = cold_logger


def answer(step: Step) -> JsonValue:
    if step["operation"] == "detail":
        return detail(step)
    if step["operation"] == "metadata":
        return metadata(step)
    return daily_summary(step)


def detail(step: DetailStep) -> JsonValue:
    stored = step["row"]
    row: dict[str, JsonValue] = {
        "request_id": stored["request_id"],
        "messages": stored["messages"],
        "response": stored["response"],
        "proxy_server_request": stored["proxy_server_request"],
        "metadata": stored["metadata"],
        "user": None,
        "team_id": None,
    }
    proxy_server.prisma_client = Client(Db([row]))
    cold = ColdLogger(step["cold"])
    litellm.logging_callback_manager = Loggers(cold)
    litellm.cold_storage_custom_logger = COLD_LOGGER
    response = TestClient(App()).get(f"/spend/logs/ui/{stored['request_id']}")
    response.raise_for_status()
    keys: list[JsonValue] = [*cold.keys]
    return {"body": BODY.validate_json(response.content), "cold_keys": keys}


def metadata(step: MetadataStep) -> JsonValue:
    rows: list[dict[str, JsonValue]] = [
        {"request_id": f"req-{index}", "metadata": value}
        for index, value in enumerate(step["values"])
    ]
    proxy_server.prisma_client = Client(Db(rows))
    response = TestClient(App()).get("/spend/logs/session/ui", params={"session_id": "session-1"})
    response.raise_for_status()
    page = PAGE.validate_json(response.content)
    hydrated: list[JsonValue] = [row["metadata"] for row in page["data"]]
    return {"metadata": hydrated}


def daily_summary(step: SummaryStep) -> JsonValue:
    rows: list[dict[str, JsonValue]] = [
        {
            "day": row["day"],
            "api_key": row["api_key"],
            "user": row["user"],
            "model": row["model"],
            "spend": row["spend"],
        }
        for row in step["rows"]
    ]
    db = Db(rows)
    proxy_server.prisma_client = Client(db)
    filters: dict[str, str | None] = {
        "api_key": step["api_key"],
        "request_id": step["request_id"],
        "user_id": step["user_id"],
    }
    query: dict[str, str] = {"start_date": step["start_date"], "end_date": step["end_date"]}
    query |= {name: value for name, value in filters.items() if value is not None}
    response = TestClient(App()).get(f"/spend/logs?{urlencode(query)}")
    response.raise_for_status()
    sql, params = db.queries[-1]
    return {"sql": sql, "params": params, "body": BODY.validate_json(response.content)}
