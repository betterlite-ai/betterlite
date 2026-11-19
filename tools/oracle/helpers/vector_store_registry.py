import asyncio
from typing import Generic, Literal, TypedDict, TypeVar

import litellm
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.utils import CredentialItem
from litellm.types.vector_stores import (
    LiteLLM_ManagedVectorStore,
    LiteLLM_ManagedVectorStoreIndex as ManagedIndex,
)
from litellm.vector_stores.vector_store_registry import (
    VectorStoreIndexRegistry,
    VectorStoreRegistry,
)
from pydantic import JsonValue, TypeAdapter

CREDENTIALS: TypeAdapter[list[CredentialItem]] = TypeAdapter(list[CredentialItem])
JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
Row = TypeVar("Row")


class Step(TypedDict, total=False):
    kind: Literal["add", "update", "delete", "is_config", "upsert_index", "lookup_index", "is_index", "defaults"]
    store: LiteLLM_ManagedVectorStore
    id: str
    index: dict[str, JsonValue]
    name: str


class DatabaseFields(TypedDict, total=False):
    stores: list[LiteLLM_ManagedVectorStore]
    indexes: list[dict[str, JsonValue]]
    failing: bool


class Input(TypedDict, total=False):
    operation: Literal[
        "config_records", "mutations", "database_lookup",
        "database_rows", "ids_and_tool_params", "named_credentials",
    ]
    stores: list[LiteLLM_ManagedVectorStore]
    indexes: list[dict[str, JsonValue]]
    entries: list[dict[str, JsonValue]]
    steps: list[Step]
    lookups: list[str]
    database: DatabaseFields
    non_default_params: dict[str, JsonValue]
    tools: list[dict[str, JsonValue]] | None
    tool: dict[str, JsonValue]
    credentials: dict[str, dict[str, JsonValue]]
    ids: list[str]


class FakeTable(Generic[Row]):
    def __init__(self, name: str, rows: list[Row], calls: list[JsonValue], *, failing: bool) -> None:
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
        stores: FakeTable[LiteLLM_ManagedVectorStore],
        indexes: FakeTable[dict[str, JsonValue]],
    ) -> None:
        self.litellm_managedvectorstorestable = stores
        self.litellm_managedvectorstoreindextable = indexes


class FakeClient(PrismaClient):
    def __init__(self, database: DatabaseFields, calls: list[JsonValue]) -> None:
        stores = database.get("stores")
        indexes = database.get("indexes")
        failing = database.get("failing")
        fails = False if failing is None else failing
        self.db = FakeDb(
            FakeTable("stores", [] if stores is None else stores, calls, failing=fails),
            FakeTable("indexes", [] if indexes is None else indexes, calls, failing=fails),
        )


def store_value(store: LiteLLM_ManagedVectorStore) -> dict[str, JsonValue]:
    return {
        "vector_store_id": store.get("vector_store_id"),
        "custom_llm_provider": store.get("custom_llm_provider"),
        "vector_store_name": store.get("vector_store_name"),
        "vector_store_description": store.get("vector_store_description"),
        "vector_store_metadata": JSON_VALUE.validate_python(store.get("vector_store_metadata")),
        "litellm_credential_name": store.get("litellm_credential_name"),
        "litellm_params": JSON_VALUE.validate_python(store.get("litellm_params")),
        "team_id": store.get("team_id"),
        "user_id": store.get("user_id"),
        "is_config": store.get("is_config"),
    }


def index_value(index: ManagedIndex) -> dict[str, JsonValue]:
    return {
        "id": index.id,
        "index_name": index.index_name,
        "litellm_params": {
            "vector_store_index": index.litellm_params.vector_store_index,
            "vector_store_name": index.litellm_params.vector_store_name,
        },
        "index_info": JSON_VALUE.validate_python(index.index_info),
        "created_by": index.created_by,
        "updated_by": index.updated_by,
    }


def step_value(step: Step, registry: VectorStoreRegistry, indexes: VectorStoreIndexRegistry) -> JsonValue:
    result: JsonValue = None
    match step["kind"]:
        case "add":
            registry.add_vector_store_to_registry(vector_store=step["store"])
            result = {"stores": [store_value(store) for store in registry.vector_stores]}
        case "update":
            registry.update_vector_store_in_registry(vector_store_id=step["id"], updated_data=step["store"])
            result = {"stores": [store_value(store) for store in registry.vector_stores]}
        case "delete":
            registry.delete_vector_store_from_registry(vector_store_id=step["id"])
            result = {"stores": [store_value(store) for store in registry.vector_stores]}
        case "is_config":
            result = {"is_config": registry.is_config_vector_store(step["id"])}
        case "upsert_index":
            indexes.upsert_vector_store_index(ManagedIndex.model_validate(step["index"]))
            result = {"indexes": [index_value(index) for index in indexes.vector_store_indexes]}
        case "lookup_index":
            found = indexes.get_vector_store_index_by_name(step["name"])
            result = {"index": None if found is None else index_value(found)}
        case "is_index":
            result = {"is_index": indexes.is_vector_store_index(step["name"])}
        case _:
            result = {
                "stores": [
                    store_value(store) for store in VectorStoreRegistry().vector_stores
                ],
                "indexes": [
                    index_value(index)
                    for index in VectorStoreIndexRegistry().vector_store_indexes
                ],
            }
    return result


def config_records(input_value: Input) -> JsonValue:
    registry = VectorStoreRegistry()
    raised: str | None = None
    try:
        registry.load_vector_stores_from_config(input_value["entries"])
    except ValueError as error:
        raised = str(error)
    return {"stores": [store_value(store) for store in registry.vector_stores], "raised": raised}


def mutations(input_value: Input) -> JsonValue:
    registry = VectorStoreRegistry(vector_stores=input_value["stores"])
    indexes = VectorStoreIndexRegistry(
        vector_store_indexes=[
            ManagedIndex.model_validate(index) for index in input_value["indexes"]
        ]
    )
    results = [step_value(step, registry, indexes) for step in input_value["steps"]]
    return {"results": results}


def database_lookup(input_value: Input) -> JsonValue:
    registry = VectorStoreRegistry(vector_stores=input_value["stores"])
    calls: list[JsonValue] = []
    database = input_value.get("database")
    client = None if database is None else FakeClient(database, calls)
    results: list[JsonValue] = []
    for lookup in input_value["lookups"]:
        found = asyncio.run(
            registry.get_litellm_managed_vector_store_from_registry_or_db(
                vector_store_id=lookup, prisma_client=client
            )
        )
        results.append(None if found is None else store_value(found))
    return {"results": results, "database_calls": calls}


def database_rows(input_value: Input) -> JsonValue:
    calls: list[JsonValue] = []
    database = input_value.get("database")
    client = None if database is None else FakeClient(database, calls)
    stores: list[JsonValue] = []
    stores_failed = False
    try:
        rows = asyncio.run(VectorStoreRegistry._get_vector_stores_from_db(client))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L495-L511
        stores = [store_value(store) for store in rows]
    except RuntimeError:
        stores_failed = True
    indexes: list[JsonValue] = []
    indexes_failed = False
    try:
        index_rows = asyncio.run(VectorStoreIndexRegistry._get_vector_store_indexes_from_db(client))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L84-L100
        indexes = [index_value(index) for index in index_rows]
    except RuntimeError:
        indexes_failed = True
    return {"stores": stores, "stores_failed": stores_failed,
            "indexes": indexes, "indexes_failed": indexes_failed,
            "database_calls": calls}


def ids_and_tool_params(input_value: Input) -> JsonValue:
    registry = VectorStoreRegistry()
    ids = JSON_VALUE.validate_python(
        registry.get_vector_store_ids_to_run(
            non_default_params=input_value["non_default_params"],
            tools=input_value["tools"],
        )
    )
    raised: str | None = None
    params: dict[str, JsonValue] | None = None
    try:
        tool_params = registry._extract_tool_params(input_value["tool"])  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/vector_stores/vector_store_registry.py#L108-L121
        params = {
            "filters": JSON_VALUE.validate_python(tool_params.filters),
            "max_num_results": tool_params.max_num_results,
            "ranking_options": JSON_VALUE.validate_python(tool_params.ranking_options),
        }
    except TypeError as error:
        raised = str(error)
    return {"ids": ids, "params": input_value["non_default_params"],
            "tools": JSON_VALUE.validate_python(input_value["tools"]),
            "tool_params": params, "raised": raised}


def named_credentials(input_value: Input) -> JsonValue:
    previous = litellm.credential_list
    try:
        records: list[dict[str, JsonValue]] = [
            {"credential_name": name, "credential_info": {}, "credential_values": values}
            for name, values in input_value["credentials"].items()
        ]
        litellm.credential_list = CREDENTIALS.validate_python(records)
        registry = VectorStoreRegistry(vector_stores=input_value["stores"])
        credentials = [
            OBJECT.validate_python(registry.get_credentials_for_vector_store(identifier))
            for identifier in input_value["ids"]
        ]
        return {"credentials": credentials}
    finally:
        litellm.credential_list = previous


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    if operation == "config_records":
        return config_records(input_value)
    if operation == "mutations":
        return mutations(input_value)
    if operation == "database_lookup":
        return database_lookup(input_value)
    if operation == "database_rows":
        return database_rows(input_value)
    if operation == "ids_and_tool_params":
        return ids_and_tool_params(input_value)
    return named_credentials(input_value)
