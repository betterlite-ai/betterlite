from logging import DEBUG, Handler, LogRecord
from types import ModuleType
from typing import Final

from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable as StoreRow
from pydantic import JsonValue
from typing_extensions import override

from b10014.case import (
    CACHE_FAILURE,
    CACHE_TTL_SECONDS,
    REGISTRY_FAILURE,
    WHERE,
    Failure,
    RowQuery,
)


# the table LiteLLM queries and validates each dumped row into, as the real repository does
class Table:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.rows = rows
        self.queries: list[RowQuery] = []

    async def find_many(self, where: dict[str, JsonValue], take: int) -> list[StoreRow]:
        uuids = WHERE.validate_python(where)["vector_store_id"]["in"]
        self.queries.append({"uuids": uuids, "take": take})
        return [StoreRow.model_validate(row) for row in self.rows]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/managed_files.py#L52


class Database:
    def __init__(self, table: Table) -> None:
        self.litellm_managedvectorstorestable = table


class Prisma:
    def __init__(self, table: Table) -> None:
        self.db = Database(table)


# the shared cache LiteLLM reads and writes one row per UUID through
class Cache:
    def __init__(self, hits: dict[str, StoreRow], failure: Failure | None) -> None:
        self.hits = hits
        self.failure = failure
        self.reads: list[str] = []
        self.writes: list[str] = []
        self.default_in_memory_ttl = CACHE_TTL_SECONDS

    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> StoreRow | None:
        self.reads.append(key)
        if self.failure == "cache_read":
            raise RuntimeError(CACHE_FAILURE)
        return self.hits.get(key)

    async def async_set_cache(self, key: str, value: StoreRow, **_kwargs: JsonValue) -> None:
        if self.failure == "cache_write":
            raise RuntimeError(CACHE_FAILURE)
        self.writes.append(key)
        self.hits[key] = value


# the registry LiteLLM asks for a store by id, answering with the first store it holds
class Registry:
    def __init__(self, stores: list[dict[str, JsonValue]], *, failing: bool) -> None:
        self.stores = stores
        self.failing = failing
        self.looks: list[str] = []

    def get_litellm_managed_vector_store_from_registry(  # ast-grep-ignore: no-long-names
        self, vector_store_id: str
    ) -> dict[str, JsonValue] | None:
        self.looks.append(vector_store_id)
        if self.failing:
            raise RuntimeError(REGISTRY_FAILURE)
        for store in self.stores:
            if store.get("vector_store_id") == vector_store_id:
                return store
        return None


# the log board the case reads warnings from, and the proxy logging object the metrics hook reaches
class Board(Handler):
    def __init__(self) -> None:
        super().__init__(DEBUG)
        self.messages: list[str] = []
        self.service_logging_obj: Final = self

    @override
    def emit(self, record: LogRecord) -> None:
        self.messages.append(record.getMessage())

    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class ProxyServer(ModuleType):
    prisma_client: Prisma | None
    user_api_key_cache: Cache | None
    proxy_logging_obj: Board
