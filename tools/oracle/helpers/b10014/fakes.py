from logging import DEBUG, Handler, LogRecord
from types import ModuleType
from typing import Final

from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable as StoreRow
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
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

# the row the scripted table hands back: LiteLLM calls model_dump on it, and the
# projection being a plain dict keeps an empty one from being skipped
class Row:
    __slots__ = ("fields",)

    def __init__(self, fields: dict[str, JsonValue]) -> None:
        self.fields = fields

    def model_dump(self) -> dict[str, JsonValue]:
        return self.fields


class Table:
    def __init__(self, rows: list[Row]) -> None:
        self.rows = rows
        self.queries: list[RowQuery] = []

    async def find_many(self, where: dict[str, JsonValue], take: int) -> list[Row]:
        uuids = WHERE.validate_python(where)["vector_store_id"]["in"]
        self.queries.append({"uuids": uuids, "take": take})
        return self.rows


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


class Registry(VectorStoreRegistry):
    def __init__(self, stores: list[LiteLLM_ManagedVectorStore], *, failing: bool) -> None:
        super().__init__(vector_stores=stores)
        self.failing = failing
        self.looks: list[str] = []

    @override
    def get_litellm_managed_vector_store_from_registry(  # ast-grep-ignore: no-long-names
        self, vector_store_id: str
    ) -> LiteLLM_ManagedVectorStore | None:
        self.looks.append(vector_store_id)
        if self.failing:
            raise RuntimeError(REGISTRY_FAILURE)
        return super().get_litellm_managed_vector_store_from_registry(vector_store_id)


# the log board the case reads its warnings from, and the proxy logging object
# the lookup's metrics hook reaches
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
