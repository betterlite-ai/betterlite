import asyncio
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.vector_stores import (
    LiteLLM_ManagedVectorStore as ManagedStore,
)
from litellm.types.vector_stores import (
    VectorStoreToolParams,
)
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter


class Store(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str
    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    litellm_credential_name: str | None
    litellm_params: JsonValue
    team_id: str | None
    user_id: str | None
    is_config: bool


class Database(TypedDict, total=False):
    rows: list[Store]
    verified: list[Store]
    verify_failed: bool
    read_failed: bool


class Input(TypedDict):
    operation: Literal["consumed", "selection", "lookup", "fallback"]
    stores: NotRequired[list[Store]]
    tools: NotRequired[list[dict[str, JsonValue]] | None]
    non_default_params: NotRequired[dict[str, JsonValue]]
    ids: NotRequired[list[str]]
    name: NotRequired[str]
    database: NotRequired[Database]


JSON_VALUE = TypeAdapter(JsonValue)
STORES = TypeAdapter(list[Store])
ONE_STORE = TypeAdapter(Store | None)
StoreList: TypeAlias = list[ManagedStore]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49


class FakeTable:
    def __init__(self, database: Database, calls: list[JsonValue]) -> None:
        rows = database.get("rows")
        verified = database.get("verified")
        verify_failed = database.get("verify_failed")
        read_failed = database.get("read_failed")
        self._rows = [] if rows is None else rows
        self._verified = [] if verified is None else verified
        self._verify_failed = False if verify_failed is None else verify_failed
        self._read_failed = False if read_failed is None else read_failed
        self._calls = calls

    async def find_many(self, order: dict[str, JsonValue]) -> list[Store]:
        self._calls.append({"table": "stores", "order": order})
        await asyncio.sleep(0)
        if self._read_failed:
            raise RuntimeError("database read failed")
        return self._rows

    async def find_unique(self, where: dict[str, JsonValue]) -> Store | None:
        self._calls.append({"table": "stores", "where": where})
        await asyncio.sleep(0)
        if self._verify_failed:
            raise RuntimeError("database verification failed")
        identifier = where["vector_store_id"]
        for row in self._verified:
            if row.get("vector_store_id") == identifier:
                return row
        return None


class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeTable) -> None:
        self.litellm_managedvectorstorestable = table


class FakeClient(PrismaClient):
    def __init__(self, database: Database, calls: list[JsonValue]) -> None:
        self.db = FakeDb(FakeTable(database, calls))


def registry_of(input_value: Input) -> VectorStoreRegistry:
    stores = input_value.get("stores")
    seed = [] if stores is None else stores
    return VectorStoreRegistry(vector_stores=cast("StoreList", seed))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49


def store_value(store: Store) -> dict[str, JsonValue]:
    return {
        "custom_llm_provider": store.get("custom_llm_provider"),
        "is_config": store.get("is_config"),
        "litellm_credential_name": store.get("litellm_credential_name"),
        "litellm_params": store.get("litellm_params"),
        "team_id": store.get("team_id"),
        "user_id": store.get("user_id"),
        "vector_store_description": store.get("vector_store_description"),
        "vector_store_id": store.get("vector_store_id"),
        "vector_store_metadata": store.get("vector_store_metadata"),
        "vector_store_name": store.get("vector_store_name"),
    }


def project(stores: list[ManagedStore]) -> list[JsonValue]:
    records = STORES.validate_python(stores)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    return [store_value(record) for record in records]


def params_value(params: VectorStoreToolParams) -> JsonValue:
    fields: dict[str, JsonValue] = {}
    filters = JSON_VALUE.validate_python(
        params.filters  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L326-L342
    )
    if filters is not None:
        fields["filters"] = filters
    results = params.max_num_results
    if results is not None:
        fields["max_num_results"] = results
    options = JSON_VALUE.validate_python(
        params.ranking_options  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L326-L342
    )
    if options is not None:
        fields["ranking_options"] = options
    return JSON_VALUE.validate_python(fields)


def consumed(input_value: Input) -> dict[str, JsonValue]:
    registry = registry_of(input_value)
    tools = input_value.get("tools")
    listed = [] if tools is None else tools
    ids = input_value.get("ids")
    identifiers = [] if ids is None else ids
    raised: str | None = None
    params: dict[str, JsonValue] = {}
    try:
        extracted = registry.get_and_pop_recognised_vector_store_tools(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L141-L186
            tools=listed, vector_store_ids=identifiers
        )
        params = {name: params_value(value) for name, value in extracted.items()}
    except TypeError as error:
        raised = str(error)
    return {
        "params": params,
        "tools": JSON_VALUE.validate_python(listed),
        "ids": JSON_VALUE.validate_python(identifiers),
        "raised": raised,
    }


def selection(input_value: Input) -> dict[str, JsonValue]:
    registry = registry_of(input_value)
    given = input_value.get("non_default_params")
    params = {} if given is None else given
    tools = input_value.get("tools")
    raised: str | None = None
    stores: list[JsonValue] = []
    try:
        stores = project(
            asyncio.run(registry.pop_vector_stores_to_run_with_db_fallback(params, tools=tools))  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L303-L396
        )
    except TypeError as error:
        raised = str(error)
    return {
        "stores": stores,
        "registry": project(registry.vector_stores),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
        "params": params,
        "tools": JSON_VALUE.validate_python(tools),
        "raised": raised,
    }


def lookup(input_value: Input) -> dict[str, JsonValue]:
    registry = registry_of(input_value)
    given = input_value.get("non_default_params")
    params = {} if given is None else given
    tools = input_value.get("tools")
    name = input_value.get("name")
    first = ONE_STORE.validate_python(
        registry.get_vector_store_to_run(params, tools=tools)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L189-L205
    )
    resolved_name = "" if name is None else name
    named = ONE_STORE.validate_python(
        registry.get_litellm_managed_vector_store_from_registry_by_name(resolved_name)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L250-L259
    )
    return {
        "first": None if first is None else store_value(first),
        "named": None if named is None else store_value(named),
        "params": params,
        "tools": JSON_VALUE.validate_python(tools),
        "registry": project(registry.vector_stores),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    }


def fallback(input_value: Input) -> dict[str, JsonValue]:
    registry = registry_of(input_value)
    given = input_value.get("non_default_params")
    params = {} if given is None else given
    tools = input_value.get("tools")
    calls: list[JsonValue] = []
    database = input_value.get("database")
    db: PrismaClient | None = None if database is None else FakeClient(database, calls)
    raised: str | None = None
    stores: list[JsonValue] = []
    try:
        stores = project(
            asyncio.run(registry.pop_vector_stores_to_run_with_db_fallback(params, tools, db))  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L303-L396
        )
    except TypeError as error:
        raised = str(error)
    return {
        "stores": stores,
        "registry": project(registry.vector_stores),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
        "params": params,
        "tools": JSON_VALUE.validate_python(tools),
        "database_calls": calls,
        "raised": raised,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "consumed":
        return consumed(input_value)
    if input_value["operation"] == "selection":
        return selection(input_value)
    if input_value["operation"] == "lookup":
        return lookup(input_value)
    return fallback(input_value)
