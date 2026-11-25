import asyncio
import sys
from logging import DEBUG, Handler, LogRecord, getLogger
from types import ModuleType
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.vector_store_endpoints.utils import (
    assert_user_can_access_vector_store,
    assert_user_can_access_vector_store_id,
    get_litellm_managed_vector_store,
)
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

# the proxy module the resolution imports its shared cache from, and the cache key its lookup builds
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CACHE_KEY_PREFIX: Final = "managed_vector_store_id:"
CACHE_TTL_SECONDS: Final = 60.0

# what the scripted library calls raise, so the warning names the arm the case scripted
REGISTRY_FAILURE: Final = "the case scripts a registry failure"
CACHE_FAILURE: Final = "the case scripts a cache failure"

JSON: Final = TypeAdapter(JsonValue)
STORE: Final = TypeAdapter(dict[str, JsonValue])
DETAIL: Final = TypeAdapter(str)
AUTH: Final = TypeAdapter(UserAPIKeyAuth)
WHERE: Final = TypeAdapter(dict[str, dict[str, list[str]]])

Failure: TypeAlias = Literal["registry", "cache_read", "cache_write"]


class RowInput(TypedDict):
    model_dump: dict[str, JsonValue]


class RegistryInput(TypedDict):
    stores: NotRequired[list[LiteLLM_ManagedVectorStore]]


# the credential fields the assertions read, the same shape the native helper builds its auth from
class StoreAuth(TypedDict):
    team_id: NotRequired[str]
    user_id: NotRequired[str]
    user_role: NotRequired[str]
    object_permission_id: NotRequired[str]
    vector_stores: NotRequired[list[str]]


class ResolveCall(TypedDict):
    function: Literal["resolve"]
    vector_store_id: str
    registry: NotRequired[RegistryInput]
    database: bool
    cached: NotRequired[list[RowInput]]
    rows: NotRequired[list[RowInput]]
    fail: NotRequired[Failure]


class AssertStoreCall(TypedDict):
    function: Literal["assert_store"]
    store: LiteLLM_ManagedVectorStore
    auth: StoreAuth
    detail: NotRequired[str]


class AssertIdCall(TypedDict):
    function: Literal["assert_id"]
    vector_store_id: str
    registry: NotRequired[RegistryInput]
    database: bool
    cached: NotRequired[list[RowInput]]
    rows: NotRequired[list[RowInput]]
    fail: NotRequired[Failure]
    auth: StoreAuth
    detail: NotRequired[str]


Call: TypeAlias = ResolveCall | AssertStoreCall | AssertIdCall


class Input(TypedDict):
    calls: list[Call]


class ErrorView(TypedDict):
    status: int
    detail: str


class RowQuery(TypedDict):
    uuids: list[str]
    take: int


class Lookups(TypedDict):
    registry: list[str]
    cache_reads: list[str]
    cache_writes: list[str]
    row_queries: list[RowQuery]


class Answer(TypedDict):
    function: str
    store: dict[str, JsonValue] | None
    error: ErrorView | None
    warnings: list[str]
    lookups: Lookups


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
    def __init__(
        self, hits: dict[str, LiteLLM_ManagedVectorStoresTable], failure: Failure | None
    ) -> None:
        self.hits = hits
        self.failure = failure
        self.reads: list[str] = []
        self.writes: list[str] = []
        self.default_in_memory_ttl = CACHE_TTL_SECONDS

    async def async_get_cache(
        self, key: str, **_kwargs: JsonValue
    ) -> LiteLLM_ManagedVectorStoresTable | None:
        self.reads.append(key)
        if self.failure == "cache_read":
            raise RuntimeError(CACHE_FAILURE)
        return self.hits.get(key)

    async def async_set_cache(
        self, key: str, value: LiteLLM_ManagedVectorStoresTable, **_kwargs: JsonValue
    ) -> None:
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


def permission_value(auth: StoreAuth) -> dict[str, JsonValue] | None:
    object_permission_id = auth.get("object_permission_id")
    stores = auth.get("vector_stores")
    if object_permission_id is None and stores is None:
        return None
    permission: dict[str, JsonValue] = {}
    if object_permission_id is not None:
        permission["object_permission_id"] = object_permission_id
    if stores is not None:
        permission["vector_stores"] = JSON.validate_python(stores)
    return permission


def key_auth(auth: StoreAuth) -> UserAPIKeyAuth:
    mapping: dict[str, JsonValue] = {}
    team_id = auth.get("team_id")
    if team_id is not None:
        mapping["team_id"] = team_id
    user_id = auth.get("user_id")
    if user_id is not None:
        mapping["user_id"] = user_id
    user_role = auth.get("user_role")
    if user_role is not None:
        mapping["user_role"] = user_role
    permission = permission_value(auth)
    if permission is not None:
        mapping["object_permission"] = permission
    return AUTH.validate_python(mapping)


# the store as it stands: a row carries every column, and a registry store carries
# the fields it was given
def store_view(store: LiteLLM_ManagedVectorStore | None) -> dict[str, JsonValue] | None:
    if store is None:
        return None
    return STORE.validate_python(store)


def error_view(raised: HTTPException) -> ErrorView:
    return {"status": raised.status_code, "detail": DETAIL.validate_python(raised.detail)}


def warning_labels(messages: list[str], vector_store_id: str) -> list[str]:
    registry_prefix = f"Failed to resolve vector store id={vector_store_id} from registry"
    cache_prefix = f"Failed to resolve vector store id={vector_store_id} from shared cache"
    labels: list[str] = []
    for message in messages:
        if message.startswith(registry_prefix):
            labels.append("registry")
        elif message.startswith(cache_prefix):
            labels.append("shared_cache")
    return labels


def lookups_view(registry: Registry | None, cache: Cache, table: Table | None) -> Lookups:
    return {
        "registry": [] if registry is None else registry.looks,
        "cache_reads": cache.reads,
        "cache_writes": cache.writes,
        "row_queries": [] if table is None else table.queries,
    }


def registry_value(call: ResolveCall | AssertIdCall) -> Registry | None:
    scripted = call.get("registry")
    if scripted is None:
        return None
    stores = scripted.get("stores")
    return Registry(
        [] if stores is None else stores, failing=call.get("fail") == "registry"
    )


def cache_value(call: ResolveCall | AssertIdCall) -> Cache:
    cached = call.get("cached")
    hits: dict[str, LiteLLM_ManagedVectorStoresTable] = {}
    for row in [] if cached is None else cached:
        table = LiteLLM_ManagedVectorStoresTable.model_validate(row["model_dump"])
        hits[f"{CACHE_KEY_PREFIX}{table.vector_store_id}"] = table
    return Cache(hits, call.get("fail"))


def table_value(call: ResolveCall | AssertIdCall) -> Table | None:
    if not call["database"]:
        return None
    rows = call.get("rows")
    return Table([] if rows is None else [Row(row["model_dump"]) for row in rows])


def prepare(registry: Registry | None, cache: Cache, table: Table | None, board: Board) -> None:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = None if table is None else Prisma(table)
    proxy_server.user_api_key_cache = cache
    proxy_server.proxy_logging_obj = board
    sys.modules[PROXY_MODULE] = proxy_server
    litellm.vector_store_registry = registry


async def asserted(
    store: LiteLLM_ManagedVectorStore, auth: UserAPIKeyAuth, detail: str | None
) -> None:
    if detail is None:
        await assert_user_can_access_vector_store(vector_store=store, user_api_key_dict=auth)
    else:
        await assert_user_can_access_vector_store(
            vector_store=store, user_api_key_dict=auth, detail=detail
        )


async def looked_up(call: AssertIdCall, auth: UserAPIKeyAuth) -> LiteLLM_ManagedVectorStore | None:
    detail = call.get("detail")
    if detail is None:
        return await assert_user_can_access_vector_store_id(
            vector_store_id=call["vector_store_id"], user_api_key_dict=auth
        )
    return await assert_user_can_access_vector_store_id(
        vector_store_id=call["vector_store_id"], user_api_key_dict=auth, detail=detail
    )


def resolve_answer(call: ResolveCall, board: Board) -> Answer:
    registry = registry_value(call)
    cache = cache_value(call)
    table = table_value(call)
    start = len(board.messages)
    store: dict[str, JsonValue] | None = None
    error: ErrorView | None = None
    prepare(registry, cache, table, board)
    try:
        store = store_view(
            asyncio.run(get_litellm_managed_vector_store(vector_store_id=call["vector_store_id"]))
        )
    except HTTPException as raised:
        error = error_view(raised)
    return {
        "function": "resolve",
        "store": store,
        "error": error,
        "warnings": warning_labels(board.messages[start:], call["vector_store_id"]),
        "lookups": lookups_view(registry, cache, table),
    }


def assert_store_answer(call: AssertStoreCall, board: Board) -> Answer:
    auth = key_auth(call["auth"])
    cache = Cache({}, None)
    error: ErrorView | None = None
    prepare(None, cache, None, board)
    try:
        asyncio.run(asserted(call["store"], auth, call.get("detail")))
    except HTTPException as raised:
        error = error_view(raised)
    # the store was handed to the assertion, so this call never resolves one and logs no warning
    return {
        "function": "assert_store",
        "store": None,
        "error": error,
        "warnings": [],
        "lookups": lookups_view(None, cache, None),
    }


def assert_id_answer(call: AssertIdCall, board: Board) -> Answer:
    registry = registry_value(call)
    cache = cache_value(call)
    table = table_value(call)
    auth = key_auth(call["auth"])
    start = len(board.messages)
    store: dict[str, JsonValue] | None = None
    error: ErrorView | None = None
    prepare(registry, cache, table, board)
    try:
        store = store_view(asyncio.run(looked_up(call, auth)))
    except HTTPException as raised:
        error = error_view(raised)
    return {
        "function": "assert_id",
        "store": store,
        "error": error,
        "warnings": warning_labels(board.messages[start:], call["vector_store_id"]),
        "lookups": lookups_view(registry, cache, table),
    }


def answer(call: Call, board: Board) -> Answer:
    match call["function"]:
        case "resolve":
            return resolve_answer(call, board)
        case "assert_store":
            return assert_store_answer(call, board)
        case "assert_id":
            return assert_id_answer(call, board)


def run(input_value: Input) -> JsonValue:
    board = Board()
    logger = getLogger("LiteLLM Proxy")
    logger.setLevel(DEBUG)
    logger.handlers = [board]
    answers: list[JsonValue] = [
        JSON.validate_python(answer(call, board)) for call in input_value["calls"]
    ]
    return JSON.validate_python(answers)
