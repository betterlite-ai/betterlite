import asyncio
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy.auth.auth_checks import resolve_and_validate_end_user_id
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

MANAGEMENT_TTL_SECONDS: Final = 45.0
VALIDATION_PREFIX: Final = "end_user_validation:"

EndUserAnswer = Literal["found", "missing"]
UserAnswer = Literal["found", "missing", "error"]
Table = Literal["end_user", "user", "budget"]


class Call(TypedDict):
    raw_end_user_id: NotRequired[str | None]
    key_end_user_budget_id: NotRequired[str | None]
    validate_in_db: bool
    database: bool
    proxy_default: NotRequired[str | None]
    cached: NotRequired[JsonValue]
    end_user: EndUserAnswer
    user: UserAnswer


class WriteView(TypedDict):
    key: str
    value: JsonValue
    ttl: float


class Answer(TypedDict):
    value: str | None
    reads: list[str]
    writes: list[WriteView]
    queries: list[str]


CachedValue = dict[str, JsonValue]


class Fixture(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.values: Final[dict[str, JsonValue]] = {}
        self.reads: Final[list[str]] = []
        self.writes: Final[list[WriteView]] = []
        self.queries: Final[list[str]] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue | None:
        del kwargs
        self.reads.append(key)
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: CachedValue, **kwargs: float) -> None:
        ttl = kwargs.get("ttl")
        if ttl is None:
            raise ValueError("a cache write carries a ttl")
        self.writes.append(WriteView(key=key, value=value, ttl=float(ttl)))
        self.values[key] = value

    def cache(self) -> UserApiKeyCache:
        return UserApiKeyCache(in_memory_cache=self, default_in_memory_ttl=MANAGEMENT_TTL_SECONDS)


class Row(dict[str, JsonValue]):
    def dict(self) -> dict[str, JsonValue]:
        return dict(self)


class UserRow(Row):
    organization_memberships: list[JsonValue] | None = None


class Store:
    def __init__(self, call: Call, fixture: Fixture, table: Table) -> None:
        self.call: Final = call
        self.fixture: Final = fixture
        self.table: Final = table

    async def find_unique(
        self, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> Row | None:
        del include
        if self.table == "user":
            user_id = where["user_id"]
            self.fixture.queries.append(f"user:{user_id}")
            if self.call["user"] == "error":
                raise RuntimeError("the user row read failed")
            if self.call["user"] == "missing":
                return None
            row: Final[dict[str, JsonValue]] = {
                "user_id": user_id,
                "user_email": "known@example.com",
            }
            return UserRow(row)
        if self.table == "end_user":
            end_user_id = where["user_id"]
            self.fixture.queries.append(f"end_user:{end_user_id}")
            if self.call["end_user"] == "missing":
                return None
            end_user_row: Final[dict[str, JsonValue]] = {
                "user_id": end_user_id,
                "blocked": False,
                "spend": 0.0,
            }
            return Row(end_user_row)
        budget_id = where["budget_id"]
        self.fixture.queries.append(f"budget:{budget_id}")
        return None

    async def find_first(
        self, where: dict[str, dict[str, str]], include: dict[str, bool] | None = None
    ) -> Row | None:
        del include
        email = where["user_email"]["equals"]
        self.fixture.queries.append(f"email:{email}")
        return None


class FakeDb(PrismaWrapper):
    def __init__(self, stores: dict[str, Store]) -> None:
        self._stores = stores

    @override
    def __getattr__(self, name: str) -> Store:
        try:
            return self._stores[name]
        except KeyError as error:
            raise AttributeError(name) from error


class FakePrismaClient(PrismaClient):
    def __init__(self, stores: dict[str, Store]) -> None:
        self.db = FakeDb(stores)


def seed(call: Call, fixture: Fixture) -> None:
    raw_end_user_id = call.get("raw_end_user_id")
    cached = call.get("cached")
    if raw_end_user_id is None or cached is None:
        return
    # the raw cache entry the resolver reads first, seeded as a previous request left it
    fixture.values[f"{VALIDATION_PREFIX}{raw_end_user_id}"] = cached


async def answer(call: Call) -> Answer:
    fixture = Fixture()
    client = FakePrismaClient(
        {
            "litellm_budgettable": Store(call, fixture, "budget"),
            "litellm_endusertable": Store(call, fixture, "end_user"),
            "litellm_usertable": Store(call, fixture, "user"),
        }
    )
    seed(call, fixture)
    previous_budget_id = litellm.max_end_user_budget_id
    previous_validate = litellm.validate_end_user_id_in_db
    litellm.max_end_user_budget_id = call.get("proxy_default")
    litellm.validate_end_user_id_in_db = call["validate_in_db"]
    try:
        value = await resolve_and_validate_end_user_id(
            raw_end_user_id=call.get("raw_end_user_id"),
            prisma_client=client if call["database"] else None,
            user_api_key_cache=fixture.cache(),
            key_end_user_budget_id=call.get("key_end_user_budget_id"),
        )
    finally:
        litellm.max_end_user_budget_id = previous_budget_id
        litellm.validate_end_user_id_in_db = previous_validate
    return Answer(
        value=value,
        reads=[key for key in fixture.reads if key.startswith(VALIDATION_PREFIX)],
        writes=[write for write in fixture.writes if write["key"].startswith(VALIDATION_PREFIX)],
        queries=list(fixture.queries),
    )


def run(input_value: Call) -> Answer:
    parsed = TypeAdapter(Call).validate_python(input_value)
    return asyncio.run(answer(parsed))
