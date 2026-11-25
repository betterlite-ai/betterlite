from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.vector_stores import LiteLLM_ManagedVectorStore
from pydantic import JsonValue, TypeAdapter

# the proxy module the resolution imports its shared cache from, and the cache key its lookup builds
PROXY_MODULE: Final = "litellm.proxy.proxy_server"
CACHE_KEY_PREFIX: Final = "managed_vector_store_id:"
CACHE_TTL_SECONDS: Final = 60.0

# what the scripted library calls raise, so the warning names the arm the case scripted
REGISTRY_FAILURE: Final = "the case scripts a registry failure"
CACHE_FAILURE: Final = "the case scripts a cache failure"

# LiteLLM_ManagedVectorStore's fields, with JSON in place of Any and timestamps as JSON text
class StoreProjection(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str
    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    created_at: str | None
    updated_at: str | None
    litellm_credential_name: str | None
    litellm_params: dict[str, JsonValue] | None
    team_id: str | None
    user_id: str | None
    is_config: bool


JSON: Final = TypeAdapter(JsonValue)
STORE: Final = TypeAdapter(StoreProjection)
UPSTREAM: Final = TypeAdapter(LiteLLM_ManagedVectorStore)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/vector_stores.py#L25-L49
DETAIL: Final = TypeAdapter(str)
AUTH: Final = TypeAdapter(UserAPIKeyAuth)
WHERE: Final = TypeAdapter(dict[str, dict[str, list[str]]])

Failure: TypeAlias = Literal["registry", "cache_read", "cache_write"]


class RowInput(TypedDict):
    model_dump: dict[str, JsonValue]


class RegistryInput(TypedDict):
    stores: NotRequired[list[dict[str, JsonValue]]]


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
    store: dict[str, JsonValue]
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
    store: StoreProjection | None
    error: ErrorView | None
    warnings: list[str]
    lookups: Lookups
