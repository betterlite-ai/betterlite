import asyncio
import sys

import litellm
from litellm.models.managed_files import LiteLLM_ManagedVectorStoresTable as StoreRow
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.vector_store_endpoints.utils import (
    assert_user_can_access_vector_store,
    assert_user_can_access_vector_store_id,
    get_litellm_managed_vector_store,
)
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from pydantic import JsonValue
from starlette.exceptions import HTTPException

from b10014.case import (
    AUTH,
    CACHE_KEY_PREFIX,
    DETAIL,
    JSON,
    PROXY_MODULE,
    STORE,
    Answer,
    AssertIdCall,
    AssertStoreCall,
    Call,
    ErrorView,
    Lookups,
    ResolveCall,
    StoreAuth,
)
from b10014.fakes import Board, Cache, Prisma, ProxyServer, Registry, Table


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


# the store as it stands: a row carries every column, a registry store the fields it was given
def store_view(store: LiteLLM_ManagedVectorStore | None) -> dict[str, JsonValue] | None:
    if store is None:
        return None
    return STORE.validate_python(store)


def error_view(raised: HTTPException) -> ErrorView:
    return {
        "status": raised.status_code,
        "detail": DETAIL.validate_python(raised.detail),
    }


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
    hits: dict[str, StoreRow] = {}
    for row in [] if cached is None else cached:
        table = StoreRow.model_validate(row["model_dump"])
        hits[f"{CACHE_KEY_PREFIX}{table.vector_store_id}"] = table
    return Cache(hits, call.get("fail"))


def table_value(call: ResolveCall | AssertIdCall) -> Table | None:
    if not call["database"]:
        return None
    rows = call.get("rows")
    return Table([] if rows is None else [row["model_dump"] for row in rows])


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
