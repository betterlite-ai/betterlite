import asyncio
import sys
from datetime import datetime
from types import ModuleType
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.proxy.vector_store_endpoints.utils import (
    _normalize_litellm_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L25-L37
    can_user_access_vector_store,
    filter_listable_vector_stores,
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class PermissionRow(TypedDict):
    object_permission_id: str
    vector_stores: list[str]


class TeamRow(TypedDict):
    team_id: str
    object_permission: NotRequired[PermissionRow | None]
    object_permission_id: NotRequired[str | None]


class UserRow(TypedDict):
    user_id: str
    teams: list[str]


# LiteLLM_ManagedVectorStore's fields with JSON named instead of Any, so mypy can read them
class Store(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str
    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    created_at: datetime | None
    updated_at: datetime | None
    litellm_credential_name: str | None
    litellm_params: dict[str, JsonValue] | None
    team_id: str | None
    user_id: str | None
    is_config: bool


# the credential fields the grant reads, as UserAPIKeyAuth's constructor takes them
class Auth(TypedDict):
    user_id: NotRequired[str | None]
    team_id: NotRequired[str | None]
    user_role: NotRequired[LitellmUserRoles | None]
    object_permission: NotRequired[LiteLLM_ObjectPermissionTable | None]
    object_permission_id: NotRequired[str | None]
    team_object_permission: NotRequired[LiteLLM_ObjectPermissionTable | None]
    team_object_permission_id: NotRequired[str | None]


# the proxy globals and rows a grant loads: a real database only answers with the cached rows
class World(TypedDict):
    auth: Auth
    database: bool
    object_permissions: dict[str, PermissionRow]
    teams: dict[str, TeamRow]
    user_teams: list[str]


class NormalizeCall(TypedDict):
    function: Literal["parameter_string_normalization"]
    vector_store: Store


class AccessCall(World):
    function: Literal["can_user_access_vector_store"]
    vector_store: Store


class ListCall(World):
    function: Literal["filter_listable_vector_stores"]
    vector_stores: list[Store]


Call: TypeAlias = NormalizeCall | AccessCall | ListCall


class Memory(InMemoryCache):
    def __init__(self, values: dict[str, JsonValue]) -> None:
        super().__init__()
        self.values: dict[str, JsonValue] = values

    @override
    async def async_get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.values.get(key)

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.values.get(key)


class Table:
    async def find_unique(self, **_kwargs: JsonValue) -> None: ...


# every table this policy reads is empty: a row it cannot find in the cache is absent
class Database(PrismaWrapper):
    def __init__(self) -> None:
        self.litellm_objectpermissiontable = Table()
        self.litellm_teamtable = Table()
        self.litellm_usertable = Table()


class Client(PrismaClient):
    def __init__(self) -> None:
        self.db = Database()


# log_db_metrics schedules a task on this object after each bounded read
class ProxyLogging:
    def __init__(self) -> None:
        self.service_logging_obj = self

    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class ProxyServer(ModuleType):
    prisma_client: Client | None = None
    proxy_logging_obj: ProxyLogging | None = None
    user_api_key_cache: UserApiKeyCache | None = None


def seeded(call: World) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {}
    for permission_id, permission in call["object_permissions"].items():
        values[f"object_permission_id:{permission_id}"] = JSON.validate_python(permission)
    for team_id, team in call["teams"].items():
        values[f"team_id:{team_id}"] = JSON.validate_python(team)
    user_id = call["auth"].get("user_id")
    if user_id is not None:
        values[user_id] = JSON.validate_python(
            UserRow(user_id=user_id, teams=call["user_teams"])
        )
    return values


def install(call: World) -> None:
    server = ProxyServer(PROXY_MODULE)
    server.proxy_logging_obj = ProxyLogging()
    server.user_api_key_cache = UserApiKeyCache(
        in_memory_cache=Memory(seeded(call)), default_in_memory_ttl=60.0
    )
    server.prisma_client = Client() if call["database"] else None
    sys.modules[PROXY_MODULE] = server


async def allowed(call: AccessCall) -> JsonValue:
    install(call)
    auth = UserAPIKeyAuth(**call["auth"])
    return JSON.validate_python(await can_user_access_vector_store(call["vector_store"], auth))


async def listable(call: ListCall) -> JsonValue:
    install(call)
    auth = UserAPIKeyAuth(**call["auth"])
    return JSON.validate_python(
        list(await filter_listable_vector_stores(call["vector_stores"], auth))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L246-L253
    )


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "parameter_string_normalization":
            return JSON.validate_python(
                _normalize_litellm_params(input_value["vector_store"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L25-L37
            )
        case "can_user_access_vector_store":
            return asyncio.run(allowed(input_value))
        case "filter_listable_vector_stores":
            return asyncio.run(listable(input_value))
