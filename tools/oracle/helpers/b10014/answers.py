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
    UPSTREAM,
    Answer,
    AssertIdCall,
    AssertStoreCall,
    Call,
    ErrorView,
    Lookups,
    ResolveCall,
    StoreAuth,
    StoreProjection,
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


# a row carries parsed timestamps where a registry store carries their text, so the store is serialized
# the way LiteLLM's type writes it before the projection that names its fields validates it
def store_view(store: LiteLLM_ManagedVectorStore | None) -> StoreProjection | None:
    if store is None:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
        return None
    return STORE.validate_json(UPSTREAM.dump_json(store))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49


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
        table = StoreRow.model_validate(row["model_dump"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/managed_files.py#L52
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
    litellm.vector_store_registry = registry  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1517


async def resolved(call: ResolveCall | AssertIdCall) -> StoreProjection | None:
    match call["function"]:
        case "resolve":
            return store_view(await get_litellm_managed_vector_store(vector_store_id=call["vector_store_id"]))  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L260
        case "assert_id":
            auth = key_auth(call["auth"])
            detail = call.get("detail")
            if detail is None:
                return store_view(await assert_user_can_access_vector_store_id(vector_store_id=call["vector_store_id"], user_api_key_dict=auth))  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L334
            return store_view(await assert_user_can_access_vector_store_id(vector_store_id=call["vector_store_id"], user_api_key_dict=auth, detail=detail))  # type: ignore[misc]  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L334


def resolution_answer(call: ResolveCall | AssertIdCall, board: Board) -> Answer:
    registry = registry_value(call)
    cache = cache_value(call)
    table = table_value(call)
    start = len(board.messages)
    store: StoreProjection | None = None
    error: ErrorView | None = None
    prepare(registry, cache, table, board)
    try:
        store = asyncio.run(resolved(call))
    except HTTPException as raised:
        error = error_view(raised)
    return {
        "function": call["function"],
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
        detail = call.get("detail")
        if detail is None:
            asyncio.run(assert_user_can_access_vector_store(vector_store=call["store"], user_api_key_dict=auth))  # type: ignore[arg-type]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L324
        else:
            asyncio.run(assert_user_can_access_vector_store(vector_store=call["store"], user_api_key_dict=auth, detail=detail))  # type: ignore[arg-type]  # noqa: E501  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L324
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


def answer(call: Call, board: Board) -> Answer:
    match call["function"]:
        case "resolve":
            return resolution_answer(call, board)
        case "assert_store":
            return assert_store_answer(call, board)
        case "assert_id":
            return resolution_answer(call, board)
