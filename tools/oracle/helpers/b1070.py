import sys
import types
from collections.abc import Awaitable, Callable
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar, cast, overload

import litellm
from fastapi import FastAPI, Request
from fastapi.testclient import TestClient
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.vector_store_endpoints.management_endpoints import router
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

INFO_PATH: Final = "/vector_store/info"
LIST_PATH: Final = "/vector_store/list"
V1_LIST_PATH: Final = "/v1/vector_store/list"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
DISABLE_FLAG: Final = "disable_vector_stores_for_internal_users"
BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

AuthCheck: TypeAlias = Callable[[Request], Awaitable[UserAPIKeyAuth]]
AuthAnswer: TypeAlias = Callable[[], UserAPIKeyAuth]
_Default = TypeVar("_Default")


class Caller(TypedDict):
    user_id: str
    user_role: str
    team_id: str | None


class ListCall(TypedDict):
    route: Literal["list"]
    page: int
    page_size: int
    alias: bool
    database: list[dict[str, JsonValue]] | None
    registry: list[dict[str, JsonValue]] | None
    caller: Caller
    disabled: bool


class InfoCall(TypedDict):
    route: Literal["info"]
    vector_store_id: str
    database: list[dict[str, JsonValue]] | None
    registry: list[dict[str, JsonValue]] | None
    caller: Caller
    disabled: bool


class Input(TypedDict):
    calls: list[ListCall | InfoCall]


class Answer(TypedDict):
    status: int
    body: JsonValue


# a stored row: the list route takes its dict form, the info route asks for model_dump
class StoreRow(dict[str, JsonValue]):
    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


# the managed vector stores table: the rows in given order, and the id lookup the info route makes
class StoreTable:
    def __init__(self, rows: list[StoreRow]) -> None:
        self.rows = rows

    async def find_many(self, *, order: dict[str, str]) -> list[StoreRow]:
        del order
        return self.rows

    async def find_unique(self, *, where: dict[str, str]) -> StoreRow | None:
        for row in self.rows:
            if row.get("vector_store_id") == where["vector_store_id"]:
                return row
        return None


class FakeDb:
    def __init__(self, rows: list[StoreRow]) -> None:
        self.litellm_managedvectorstorestable = StoreTable(rows)


class FakePrisma:
    def __init__(self, rows: list[StoreRow]) -> None:
        self.db = FakeDb(rows)


# the proxy module both endpoints read their database and feature flags from
class ProxyServer(types.ModuleType):
    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.prisma_client: FakePrisma | None = None
        self.general_settings: dict[str, bool] = {}
        self.user_api_key_cache: UserApiKeyCache | None = None


PROXY: Final = ProxyServer(PROXY_MODULE)
sys.modules[PROXY_MODULE] = PROXY


# the lookup FastAPI makes for the key check, answered for every callback the routes declare
class Overrides(dict[AuthCheck, AuthAnswer]):
    def __init__(self, answer: AuthAnswer) -> None:
        super().__init__()
        self.answer = answer

    def __bool__(self) -> bool:
        return True

    @overload
    def get(self, key: AuthCheck, default: None = None, /) -> AuthAnswer | None: ...

    @overload
    def get(self, key: AuthCheck, default: AuthAnswer, /) -> AuthAnswer: ...

    @overload
    def get(self, key: AuthCheck, default: _Default, /) -> AuthAnswer | _Default: ...

    @override
    def get(self, key: AuthCheck, default: _Default | None = None, /) -> AuthAnswer | _Default:
        del key, default
        return self.answer


# the management routes with the call's caller in place of the key check
class App(FastAPI):
    dependency_overrides: dict[AuthCheck, AuthAnswer]

    def __init__(self, caller: UserAPIKeyAuth) -> None:
        super().__init__()
        self.caller = caller
        self.dependency_overrides = Overrides(self.auth)
        self.include_router(router)

    def auth(self) -> UserAPIKeyAuth:
        return self.caller


def caller_of(caller: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=LitellmUserRoles(caller["user_role"]),
        team_id=caller["team_id"],
    )


def list_url(call: ListCall) -> str:
    if call["alias"]:
        return f"{V1_LIST_PATH}?page={call['page']}&page_size={call['page_size']}"
    return f"{LIST_PATH}?page={call['page']}&page_size={call['page_size']}"


def ask(caller: UserAPIKeyAuth, method: str, url: str, payload: dict[str, str] | None) -> Answer:
    client = TestClient(App(caller), raise_server_exceptions=False)
    response = client.request(method, url, json=payload)
    return {"status": response.status_code, "body": BODY.validate_json(response.content)}


# the cached records the call starts with: a list read fills the registry from its own database rows
def stage_registry(rows: list[dict[str, JsonValue]], caller: UserAPIKeyAuth) -> None:
    PROXY.general_settings = {DISABLE_FLAG: False}
    PROXY.prisma_client = FakePrisma([StoreRow(row) for row in rows])
    client = TestClient(App(caller), raise_server_exceptions=False)
    client.request("GET", LIST_PATH)


def answer_for(call: ListCall | InfoCall) -> Answer:
    caller = caller_of(call["caller"])
    registry = call["registry"]
    # a fresh list per call: LiteLLM appends the staged rows to the list it is handed
    litellm.vector_store_registry = (
        VectorStoreRegistry(vector_stores=cast("list[LiteLLM_ManagedVectorStore]", []))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
        if registry is not None
        else None
    )
    if registry is not None:
        stage_registry(registry, caller)
    PROXY.general_settings = {DISABLE_FLAG: call["disabled"]}
    database = call["database"]
    PROXY.prisma_client = None if database is None else FakePrisma(
        [StoreRow(row) for row in database]
    )
    if call["route"] == "info":
        return ask(caller, "POST", INFO_PATH, {"vector_store_id": call["vector_store_id"]})
    return ask(caller, "GET", list_url(call), None)


def run(input_value: Input) -> list[Answer]:
    return [
        answer_for(call) for call in input_value["calls"]
    ]
