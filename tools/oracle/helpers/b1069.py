import json
import sys
import types
from typing import Final, Literal, TypedDict

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
from typing_extensions import override

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
REGISTRY_ROWS: Final[TypeAdapter[list[LiteLLM_ManagedVectorStore]]] = TypeAdapter(
    list[LiteLLM_ManagedVectorStore]
)

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
        self.events.append(f"db:find:{vector_store_id}")
        for row in self.rows:
            if row.get("vector_store_id") == vector_store_id:
                return row
        return None

    async def create(self, *, data: dict[str, JsonValue]) -> StoreRow:
        row = created_row(data)
        self.events.append(f"db:create:{row['vector_store_id']}")
        self.rows.append(row)
        return row

    async def update(
        self, *, where: dict[str, str], data: dict[str, JsonValue]
    ) -> StoreRow | None:
        vector_store_id = where["vector_store_id"]
        self.events.append(f"db:update:{vector_store_id}")
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
        self.events.append(f"db:delete:{vector_store_id}")
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


# the proxy module both endpoints read their database, registry and feature flags from
class ProxyServer(types.ModuleType):
    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.prisma_client: FakePrisma | None = None
        self.general_settings: dict[str, bool] = {}
        self.user_api_key_cache: UserApiKeyCache | None = None
        self.redis_usage_cache: object = None


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
    app.dependency_overrides[user_api_key_auth] = CallerAnswer(caller)
    app.include_router(router)
    return app


# the real registry, with the calls the endpoints make recorded in the answer's order
class LoggingRegistry(VectorStoreRegistry):
    def __init__(self, events: list[str], vector_stores: list[LiteLLM_ManagedVectorStore]) -> None:
        super().__init__(vector_stores=vector_stores)
        self.events = events

    @override
    def get_litellm_managed_vector_store_from_registry(
        self, vector_store_id: str
    ) -> LiteLLM_ManagedVectorStore | None:
        self.events.append(f"cache:find:{vector_store_id}")
        return super().get_litellm_managed_vector_store_from_registry(vector_store_id)

    @override
    def add_vector_store_to_registry(self, vector_store: LiteLLM_ManagedVectorStore) -> None:
        self.events.append(f"cache:add:{vector_store.get('vector_store_id')}")
        super().add_vector_store_to_registry(vector_store)

    @override
    def delete_vector_store_from_registry(self, vector_store_id: str) -> None:
        self.events.append(f"cache:remove:{vector_store_id}")
        super().delete_vector_store_from_registry(vector_store_id)

    @override
    def update_vector_store_in_registry(
        self, vector_store_id: str, updated_data: LiteLLM_ManagedVectorStore
    ) -> None:
        self.events.append(f"cache:update:{vector_store_id}")
        super().update_vector_store_in_registry(
            vector_store_id=vector_store_id, updated_data=updated_data
        )

    # the ownership check reads the registry without a recorded lookup, as betterlite's does
    @override
    def is_config_vector_store(self, vector_store_id: str) -> bool:
        return any(
            store.get("vector_store_id") == vector_store_id and store.get("is_config", False)
            for store in self.vector_stores
        )


def caller_of(caller: Caller) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=LitellmUserRoles(caller["user_role"]),
        team_id=caller["team_id"],
    )


def registry_of(
    rows: list[dict[str, JsonValue]] | None, events: list[str]
) -> LoggingRegistry | None:
    if rows is None:
        return None
    stores = REGISTRY_ROWS.validate_json(json.dumps(rows))
    return LoggingRegistry(events, stores)


# both helpers format JSON text differently, so a column holding JSON text is compared as that JSON,
# and a column a row leaves out compares the same as one holding null
def canonical(value: JsonValue) -> JsonValue:
    if isinstance(value, dict):
        fields: dict[str, JsonValue] = value
        out: dict[str, JsonValue] = {}
        for key, item in fields.items():
            if item is None:
                continue
            if key in JSON_TEXT_FIELDS and isinstance(item, str):
                out[key] = BODY.validate_json(item)
                continue
            out[key] = canonical(item)
        return out
    if isinstance(value, list):
        items: list[JsonValue] = value
        return [canonical(item) for item in items]
    return value


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


def answer_for(call: Call) -> Answer:
    caller = caller_of(call["caller"])
    events: list[str] = []
    registry = registry_of(call["registry"], events)
    litellm.vector_store_registry = registry
    PROXY.general_settings = {DISABLE_FLAG: call["disabled"]}
    database = call["database"]
    table = None if database is None else StoreTable([StoreRow(row) for row in database], events)
    PROXY.prisma_client = None if table is None else FakePrisma(table)
    answer = ask(caller, ROUTES[call["route"]], call["body"])
    if table is not None:
        answer["database"] = canonical(BODY.validate_json(json.dumps(table.rows)))
    if registry is not None:
        answer["cache"] = canonical(BODY.validate_json(json.dumps(registry.vector_stores)))
    answer["writes"] = events
    return answer


def run(input_value: Input) -> list[JsonValue]:
    return [
        canonical(BODY.validate_json(json.dumps(answer_for(call))))
        for call in input_value["calls"]
    ]
