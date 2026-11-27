import json
import sys
import types
from typing import Final, Literal, TypedDict, cast

import litellm
from fastapi import FastAPI
from fastapi.testclient import TestClient
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.auth.user_api_key_auth import user_api_key_auth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.vector_store_endpoints.management_endpoints import router
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter

NEW_PATH: Final = "/vector_store/new"
DELETE_PATH: Final = "/vector_store/delete"
UPDATE_PATH: Final = "/vector_store/update"
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
DISABLE_FLAG: Final = "disable_vector_stores_for_internal_users"
CREATED_AT: Final = "2024-01-02T03:04:05.000"
UPDATED_AT: Final = "2024-02-03T04:05:06.000"
UPDATABLE_FIELDS: Final = (
    "custom_llm_provider",
    "vector_store_name",
    "vector_store_description",
    "vector_store_metadata",
)
JSON_TEXT_FIELDS: Final = frozenset(("litellm_params", "vector_store_metadata"))
ROUTES: Final[dict[Literal["new", "delete", "update"], str]] = {
    "new": NEW_PATH,
    "delete": DELETE_PATH,
    "update": UPDATE_PATH,
}
BODY: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

class Caller(TypedDict):
    user_id: str
    user_role: str
    team_id: str | None


class Call(TypedDict):
    route: Literal["new", "delete", "update"]
    body: dict[str, JsonValue]
    database: list[dict[str, JsonValue]] | None
    registry: list[dict[str, JsonValue]] | None
    caller: Caller
    disabled: bool


class Input(TypedDict):
    calls: list[Call]


class Answer(TypedDict):
    status: int
    body: JsonValue
    database: JsonValue
    cache: JsonValue
    writes: list[str]


# a stored row: Prisma hands the endpoints a model whose model_dump is this mapping
class StoreRow(dict[str, JsonValue]):
    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


# the managed vector stores table: rows in given order, every call recorded for the answer
class StoreTable:
    def __init__(self, rows: list[StoreRow], events: list[str]) -> None:
        self.rows = rows
        self.events = events

    async def find_unique(self, *, where: dict[str, str]) -> StoreRow | None:
        vector_store_id = where["vector_store_id"]
        self.events.append("db:find")
        for row in self.rows:
            if row.get("vector_store_id") == vector_store_id:
                return row
        return None

    async def create(self, *, data: dict[str, JsonValue]) -> StoreRow:
        row = created_row(data)
        self.events.append("db:create")
        self.rows.append(row)
        return row

    async def update(
        self, *, where: dict[str, str], data: dict[str, JsonValue]
    ) -> StoreRow | None:
        vector_store_id = where["vector_store_id"]
        self.events.append("db:update")
        for row in self.rows:
            if row.get("vector_store_id") == vector_store_id:
                for key in UPDATABLE_FIELDS:
                    if key in data:
                        row[key] = data[key]
                row["updated_at"] = UPDATED_AT
                return row
        return None

    async def delete(self, *, where: dict[str, str]) -> StoreRow | None:
        vector_store_id = where["vector_store_id"]
        self.events.append("db:delete")
        for index, row in enumerate(self.rows):
            if row.get("vector_store_id") == vector_store_id:
                del self.rows[index]
                return row
        return None


def created_row(data: dict[str, JsonValue]) -> StoreRow:
    row = StoreRow()
    row["vector_store_id"] = data["vector_store_id"]
    row["custom_llm_provider"] = data["custom_llm_provider"]
    row["created_at"] = CREATED_AT
    row["updated_at"] = UPDATED_AT
    for key in (
        "vector_store_name",
        "vector_store_description",
        "vector_store_metadata",
        "litellm_credential_name",
        "litellm_params",
        "team_id",
        "user_id",
    ):
        if key in data:
            row[key] = data[key]
    return row


class FakeDb:
    def __init__(self, table: StoreTable) -> None:
        self.litellm_managedvectorstorestable = table


class FakePrisma:
    def __init__(self, table: StoreTable) -> None:
        self.db = FakeDb(table)


# the proxy module both endpoints read their database and feature flags from
class ProxyServer(types.ModuleType):
    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.prisma_client: FakePrisma | None = None
        self.general_settings: dict[str, bool] = {}
        self.user_api_key_cache: UserApiKeyCache | None = None


PROXY: Final = ProxyServer(PROXY_MODULE)
sys.modules[PROXY_MODULE] = PROXY


# the key check the routes declare, answered with the call's caller
class CallerAnswer:
    def __init__(self, caller: UserAPIKeyAuth) -> None:
        self.caller = caller

    def __call__(self) -> UserAPIKeyAuth:
        return self.caller


# the management routes with the call's caller in place of the key check
def app_for(caller: UserAPIKeyAuth) -> FastAPI:
    app = FastAPI()
    app.dependency_overrides[user_api_key_auth] = CallerAnswer(caller)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L3432
    app.include_router(router)
    return app


def caller_of(caller: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=LitellmUserRoles(caller["user_role"]),
        team_id=caller["team_id"],
    )


# the real registry, seeded with the call's rows; the routes then change it through its own methods
def registry_of(
    rows: list[dict[str, JsonValue]] | None,
) -> VectorStoreRegistry | None:
    if rows is None:
        return None
    return VectorStoreRegistry(vector_stores=cast("list[LiteLLM_ManagedVectorStore]", rows))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49


# JSON text columns compare as the JSON they hold; a column a row leaves out compares as absent
def canonical(value: JsonValue) -> JsonValue:
    match value:
        case dict():
            fields: dict[str, JsonValue] = value
            return canonical_fields(fields)
        case list():
            return [canonical(item) for item in value]
        case None | bool() | int() | float() | str():
            return value


def canonical_fields(fields: dict[str, JsonValue]) -> JsonValue:
    out: dict[str, JsonValue] = {}
    for key, item in fields.items():
        if item is not None:
            out[key] = canonical_member(key, item)
    return out


def canonical_member(key: str, item: JsonValue) -> JsonValue:
    match item:
        case str() if key in JSON_TEXT_FIELDS:
            return BODY.validate_json(item)
        case dict() | list() | None | bool() | int() | float() | str():
            return canonical(item)


def ask(caller: UserAPIKeyAuth, path: str, payload: dict[str, JsonValue]) -> Answer:
    client = TestClient(app_for(caller), raise_server_exceptions=False)
    response = client.request("POST", path, json=payload)
    return {
        "status": response.status_code,
        "body": BODY.validate_json(response.content),
        "database": None,
        "cache": None,
        "writes": [],
    }


def cache_answer(registry: VectorStoreRegistry) -> JsonValue:
    stores = cast("list[dict[str, JsonValue]]", registry.vector_stores)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L215
    return canonical(BODY.validate_json(json.dumps(stores)))


def answer_for(call: Call) -> Answer:
    caller = caller_of(call["caller"])
    events: list[str] = []
    registry = registry_of(call["registry"])
    litellm.vector_store_registry = registry
    PROXY.general_settings = {DISABLE_FLAG: call["disabled"]}
    database = call["database"]
    table = None if database is None else StoreTable([StoreRow(row) for row in database], events)
    PROXY.prisma_client = None if table is None else FakePrisma(table)
    answer = ask(caller, ROUTES[call["route"]], call["body"])
    if table is not None:
        answer["database"] = canonical(BODY.validate_json(json.dumps(table.rows)))
    if registry is not None:
        answer["cache"] = cache_answer(registry)
    answer["writes"] = events
    return answer


def run(input_value: Input) -> list[JsonValue]:
    return [
        canonical(BODY.validate_json(json.dumps(answer_for(call))))
        for call in input_value["calls"]
    ]
