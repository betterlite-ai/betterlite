import asyncio
from typing import Generic, Literal, TypedDict, TypeVar, cast

import litellm
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.utils import CredentialItem
from litellm.types.vector_stores import (
    LiteLLM_ManagedVectorStore,
    LiteLLM_ManagedVectorStoreIndex as ManagedIndex,
    VectorStoreToolParams,
)
from litellm.vector_stores.vector_store_registry import (
    VectorStoreIndexRegistry,
    VectorStoreRegistry,
)
from pydantic import JsonValue, TypeAdapter

Row = TypeVar("Row")


class Store(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str
    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    litellm_credential_name: str | None
    litellm_params: dict[str, JsonValue] | None
    team_id: str | None
    user_id: str | None
    is_config: bool


class Database(TypedDict, total=False):
    stores: list[Store]
    indexes: list[dict[str, JsonValue]]
    failing: bool


class Step(TypedDict):
    kind: Literal[
        "add", "update", "delete", "is_config",
        "upsert_index", "lookup_index", "is_index", "defaults",
    ]
    store: Store
    id: str
    index: dict[str, JsonValue]
    name: str


class Input(TypedDict):
    operation: Literal[
        "config_records", "mutations", "database_lookup",
        "database_rows", "ids_and_tool_params", "named_credentials",
    ]
    stores: list[Store]
    indexes: list[dict[str, JsonValue]]
    entries: list[dict[str, JsonValue]]
    steps: list[Step]
    lookups: list[str]
    database: Database
    non_default_params: dict[str, JsonValue]
    tools: list[dict[str, JsonValue]] | None
    tool: dict[str, JsonValue]
    credentials: dict[str, dict[str, JsonValue]]
    ids: list[str]


JSON_VALUE = TypeAdapter(JsonValue)
STORES = TypeAdapter(list[Store])


class FakeTable(Generic[Row]):
    def __init__(
        self, name: str, rows: list[Row], calls: list[JsonValue], *, failing: bool
    ) -> None:
        self._name = name
        self._rows = rows
        self._calls = calls
        self._failing = failing

    async def find_many(self, order: dict[str, JsonValue]) -> list[Row]:
        self._calls.append({"table": self._name, "order": order})
        await asyncio.sleep(0)
        if self._failing:
            raise RuntimeError("database read failed")
        return self._rows


class FakeDb(PrismaWrapper):
    def __init__(
        self,
        stores: FakeTable[Store],
        indexes: FakeTable[dict[str, JsonValue]],
    ) -> None:
        self.litellm_managedvectorstorestable = stores
        self.litellm_managedvectorstoreindextable = indexes


class FakeClient(PrismaClient):
    def __init__(self, database: Database, calls: list[JsonValue]) -> None:
        stores = database.get("stores")
        indexes = database.get("indexes")
        failing = database.get("failing")
        fails = False if failing is None else failing
        self.db = FakeDb(
            FakeTable("stores", [] if stores is None else stores, calls, failing=fails),
            FakeTable("indexes", [] if indexes is None else indexes, calls, failing=fails),
        )


def store_value(store: Store) -> dict[str, JsonValue]:
    return {
        "vector_store_id": store.get("vector_store_id"),
        "custom_llm_provider": store.get("custom_llm_provider"),
        "vector_store_name": store.get("vector_store_name"),
        "vector_store_description": store.get("vector_store_description"),
        "vector_store_metadata": store.get("vector_store_metadata"),
        "litellm_credential_name": store.get("litellm_credential_name"),
        "litellm_params": store.get("litellm_params"),
        "team_id": store.get("team_id"),
        "user_id": store.get("user_id"),
        "is_config": store.get("is_config"),
    }


def index_value(index: ManagedIndex) -> dict[str, JsonValue]:
    params = index.litellm_params
    return {"id": index.id, "index_name": index.index_name,
            "litellm_params": {"vector_store_index": params.vector_store_index,
                               "vector_store_name": params.vector_store_name},
            "index_info": JSON_VALUE.validate_python(index.index_info),
            "created_by": index.created_by, "updated_by": index.updated_by}


def step_value(
    step: Step, registry: VectorStoreRegistry, indexes: VectorStoreIndexRegistry
) -> dict[str, JsonValue]:
    match step["kind"]:
        case "add":
            registry.add_vector_store_to_registry(vector_store=step["store"])
        case "update":
            registry.update_vector_store_in_registry(
                vector_store_id=step["id"], updated_data=step["store"]
            )
        case "delete":
            registry.delete_vector_store_from_registry(vector_store_id=step["id"])
        case "is_config":
            return {"is_config": registry.is_config_vector_store(step["id"])}
        case "upsert_index":
            indexes.upsert_vector_store_index(ManagedIndex.model_validate(step["index"]))
            listed: list[JsonValue] = [index_value(index) for index in indexes.vector_store_indexes]
            return {"indexes": listed}
        case "lookup_index":
            registered = indexes.get_vector_store_index_by_name(step["name"])
            return {"index": None if registered is None else index_value(registered)}
        case "is_index":
            return {"is_index": indexes.is_vector_store_index(step["name"])}
        case "defaults":
            return {"stores": [], "indexes": []}
    records = STORES.validate_python(registry.vector_stores)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    stores: list[JsonValue] = [store_value(store) for store in records]
    return {"stores": stores}


def config_records(input_value: Input) -> dict[str, JsonValue]:
    registry = VectorStoreRegistry()
    raised: str | None = None
    try:
        registry.load_vector_stores_from_config(input_value["entries"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L399-L436
    except ValueError as error:
        raised = str(error)
    records = STORES.validate_python(registry.vector_stores)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    stores: list[JsonValue] = [store_value(store) for store in records]
    return {"stores": stores, "raised": raised}


def mutations(input_value: Input) -> dict[str, JsonValue]:
    seed = input_value.get("stores")
    seeded = cast("list[LiteLLM_ManagedVectorStore]", [] if seed is None else seed)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    registry = VectorStoreRegistry(vector_stores=seeded)
    registered = input_value.get("indexes")
    listed = [] if registered is None else registered
    models = [ManagedIndex.model_validate(index) for index in listed]
    indexes = VectorStoreIndexRegistry(vector_store_indexes=models)
    results: list[JsonValue] = [
        step_value(step, registry, indexes) for step in input_value["steps"]
    ]
    return {"results": results}


def database_lookup(input_value: Input) -> dict[str, JsonValue]:
    seed = input_value.get("stores")
    seeded = cast("list[LiteLLM_ManagedVectorStore]", [] if seed is None else seed)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
    registry = VectorStoreRegistry(vector_stores=seeded)
    calls: list[JsonValue] = []
    database = input_value.get("database")
    db: PrismaClient | None = None if database is None else FakeClient(database, calls)
    results: list[JsonValue] = []
    for lookup in input_value["lookups"]:
        found = TypeAdapter(Store | None).validate_python(
            asyncio.run(
                registry.get_litellm_managed_vector_store_from_registry_or_db(lookup, db)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L224-L248
            )
        )
        results.append(None if found is None else store_value(found))
    return {"results": results, "database_calls": calls}


def database_rows(input_value: Input) -> dict[str, JsonValue]:
    calls: list[JsonValue] = []
    database = input_value.get("database")
    db: PrismaClient | None = None if database is None else FakeClient(database, calls)
    stores: list[JsonValue] = []
    stores_failed = False
    try:
        records = STORES.validate_python(
            asyncio.run(VectorStoreRegistry._get_vector_stores_from_db(db))  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L495-L511
        )
        stores = [store_value(store) for store in records]
    except RuntimeError:
        stores_failed = True
    indexes: list[JsonValue] = []
    indexes_failed = False
    try:
        index_rows = asyncio.run(VectorStoreIndexRegistry._get_vector_store_indexes_from_db(db))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L84-L100
        indexes = [index_value(index) for index in index_rows]
    except RuntimeError:
        indexes_failed = True
    return {"stores": stores, "stores_failed": stores_failed, "indexes": indexes,
            "indexes_failed": indexes_failed, "database_calls": calls}


def params_record(registry: VectorStoreRegistry, tool: dict[str, JsonValue]) -> JsonValue:
    params = registry._extract_tool_params(tool)  # noqa: SLF001  # pyright: ignore[reportUnknownMemberType, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L108-L121
    return JSON_VALUE.validate_python(
        {
            "filters": JSON_VALUE.validate_python(
                params.filters  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L326-L340
            ),
            "max_num_results": params.max_num_results,
            "ranking_options": JSON_VALUE.validate_python(
                params.ranking_options  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L326-L340
            ),
        }
    )


def ids_and_tool_params(input_value: Input) -> dict[str, JsonValue]:
    registry = VectorStoreRegistry()
    ids = JSON_VALUE.validate_python(
        registry.get_vector_store_ids_to_run(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L123-L139
            non_default_params=input_value["non_default_params"],
            tools=input_value["tools"],
        )
    )
    raised: str | None = None
    params: JsonValue = None
    try:
        params = params_record(registry, input_value["tool"])
    except TypeError as error:
        raised = str(error)
    return {"ids": ids, "params": input_value["non_default_params"],
            "tools": JSON_VALUE.validate_python(input_value["tools"]), "tool_params": params,
            "raised": raised}


def named_credentials(input_value: Input) -> dict[str, JsonValue]:
    records: list[dict[str, JsonValue]] = [
        {"credential_name": name, "credential_info": {}, "credential_values": values}
        for name, values in input_value["credentials"].items()
    ]
    previous = litellm.credential_list
    litellm.credential_list = TypeAdapter(list[CredentialItem]).validate_python(records)
    try:
        seeded = cast("list[LiteLLM_ManagedVectorStore]", input_value["stores"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
        registry = VectorStoreRegistry(vector_stores=seeded)
        credentials: list[JsonValue] = [
            JSON_VALUE.validate_python(registry.get_credentials_for_vector_store(identifier))
            for identifier in input_value["ids"]
        ]
        return {"credentials": credentials}
    finally:
        litellm.credential_list = previous


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "config_records":
        return config_records(input_value)
    if input_value["operation"] == "mutations":
        return mutations(input_value)
    if input_value["operation"] == "database_lookup":
        return database_lookup(input_value)
    if input_value["operation"] == "database_rows":
        return database_rows(input_value)
    if input_value["operation"] == "ids_and_tool_params":
        return ids_and_tool_params(input_value)
    return named_credentials(input_value)
