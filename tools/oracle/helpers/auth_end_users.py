import asyncio
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.budget import LiteLLM_BudgetTable
from litellm.models.end_user import LiteLLM_EndUserTable
from litellm.proxy.auth.auth_checks import (
    get_default_end_user_budget,
    get_end_user_object,
    get_key_end_user_budget_id,
    get_team_member_default_budget,
    resolve_default_end_user_budget,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache, end_user_cache_key
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

MANAGEMENT_TTL_SECONDS: Final = 45.0

Function = Literal[
    "key_budget_id", "default_budget", "team_member_budget", "resolve_default", "end_user_object"
]


class Call(TypedDict):
    function: Function
    database: bool
    key_metadata: NotRequired[dict[str, JsonValue] | None]
    proxy_default: NotRequired[str | None]
    budget_id: NotRequired[str | None]
    key_budget_id: NotRequired[str | None]
    cached_budget: NotRequired[dict[str, JsonValue] | None]
    budget_row: NotRequired[dict[str, JsonValue] | None]
    fail: NotRequired[Literal["query", "decode", "end_user_query"] | None]
    end_user_id: NotRequired[str | None]
    cached_end_user: NotRequired[dict[str, JsonValue] | None]
    end_user_row: NotRequired[dict[str, JsonValue] | None]
    validate_in_db: bool


class BudgetView(TypedDict):
    budget_id: str | None
    max_budget: float | None


class EndUserView(TypedDict):
    user_id: str
    budget_id: str | None
    spend: float
    max_budget: float | None


class Answer(TypedDict):
    value: BudgetView | EndUserView | str | None
    stored: EndUserView | None
    raised: str | None
    reads: list[str]
    writes: list[tuple[str, float]]
    queries: list[str]


CachedValue = dict[str, JsonValue]
Result = BudgetView | EndUserView | str | None


def budget_view(budget: LiteLLM_BudgetTable | None) -> BudgetView | None:
    if budget is None:
        return None
    return BudgetView(budget_id=budget.budget_id, max_budget=budget.max_budget)


def end_user_view(row: LiteLLM_EndUserTable | None) -> EndUserView | None:
    if row is None:
        return None
    relation = row.litellm_budget_table
    return EndUserView(
        user_id=row.user_id,
        budget_id=row.budget_id,
        spend=row.spend,
        max_budget=None if relation is None else relation.max_budget,
    )


def stored_view(value: CachedValue | None) -> EndUserView | None:
    if value is None:
        return None
    return end_user_view(LiteLLM_EndUserTable.model_validate(value))


class Fixture(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.values: Final[dict[str, CachedValue]] = {}
        self.reads: Final[list[str]] = []
        self.writes: Final[list[tuple[str, float]]] = []
        self.queries: Final[list[str]] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> CachedValue | None:
        del kwargs
        self.reads.append(key)
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: CachedValue, **kwargs: float) -> None:
        ttl = kwargs.get("ttl")
        if ttl is None:
            raise ValueError("a management object write carries a ttl")
        self.writes.append((key, float(ttl)))
        self.values[key] = value

    def cache(self) -> UserApiKeyCache:
        return UserApiKeyCache(in_memory_cache=self, default_in_memory_ttl=MANAGEMENT_TTL_SECONDS)


class Row(dict[str, JsonValue]):
    def dict(self) -> dict[str, JsonValue]:
        return dict(self)


UNREADABLE_BUDGET: Final[dict[str, JsonValue]] = {
    "budget_id": "unreadable",
    "max_budget": "not-a-number",
}


class Store:
    def __init__(self, call: Call, fixture: Fixture, table: Literal["budget", "end_user"]) -> None:
        self.call: Final = call
        self.fixture: Final = fixture
        self.table: Final = table

    async def find_unique(
        self, where: dict[str, str], include: dict[str, bool] | None = None
    ) -> Row | None:
        del include
        self.fixture.queries.append(next(iter(where.values())))
        fail = self.call.get("fail")
        if fail == "query" or (fail == "end_user_query" and self.table == "end_user"):
            raise RuntimeError("the database query failed")
        if self.table == "budget":
            row = UNREADABLE_BUDGET if fail == "decode" else self.call.get("budget_row")
        else:
            row = self.call.get("end_user_row")
        return None if row is None else Row(row)


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
    cached_budget = call.get("cached_budget")
    if cached_budget is not None:
        # the LiteLLM model class trips mypy's Any check, so the cache key comes from the raw row
        budget_id = cached_budget.get("budget_id")
        prefix = "None" if budget_id is None else f"{budget_id}"
        fixture.values[f"default_end_user_budget:{prefix}"] = cached_budget
        fixture.values[f"team_member_default_budget:{prefix}"] = cached_budget
    cached_end_user = call.get("cached_end_user")
    if cached_end_user is not None:
        row = LiteLLM_EndUserTable.model_validate(cached_end_user)
        fixture.values[end_user_cache_key(row.user_id)] = cached_end_user


async def looked_up(call: Call, fixture: Fixture, client: FakePrismaClient) -> Result:
    database = client if call["database"] else None
    match call["function"]:
        case "key_budget_id":
            return get_key_end_user_budget_id(call.get("key_metadata"))
        case "default_budget":
            budget = await get_default_end_user_budget(
                prisma_client=database,
                user_api_key_cache=fixture.cache(),
                budget_id=call.get("budget_id"),
            )
            return budget_view(budget)
        case "team_member_budget":
            budget_id = call.get("budget_id")
            if budget_id is None:
                raise ValueError("a team-member budget lookup names a budget_id")
            member: LiteLLM_BudgetTable | None = await get_team_member_default_budget(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1511-L1512
                budget_id=budget_id,
                prisma_client=database,
                user_api_key_cache=fixture.cache(),
            )
            return budget_view(member)
        case "resolve_default":
            resolved = await resolve_default_end_user_budget(
                prisma_client=client,
                user_api_key_cache=fixture.cache(),
                key_end_user_budget_id=call.get("key_budget_id"),
            )
            return budget_view(resolved)
        case "end_user_object":
            found: LiteLLM_EndUserTable | None = await get_end_user_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1875-L1876
                end_user_id=call.get("end_user_id"),
                prisma_client=database,
                user_api_key_cache=fixture.cache(),
                key_end_user_budget_id=call.get("key_budget_id"),
            )
            return end_user_view(found)


async def perform(
    call: Call, fixture: Fixture, client: FakePrismaClient
) -> tuple[Result, str | None]:
    try:
        return await looked_up(call, fixture, client), None
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1973
        return None, type(raised).__name__


async def answer(call: Call) -> Answer:
    fixture = Fixture()
    client = FakePrismaClient(
        {
            "litellm_budgettable": Store(call, fixture, "budget"),
            "litellm_endusertable": Store(call, fixture, "end_user"),
        }
    )
    seed(call, fixture)
    previous_budget_id = litellm.max_end_user_budget_id
    previous_validate = litellm.validate_end_user_id_in_db
    litellm.max_end_user_budget_id = call.get("proxy_default")
    litellm.validate_end_user_id_in_db = call["validate_in_db"]
    try:
        value, raised = await perform(call, fixture, client)
    finally:
        litellm.max_end_user_budget_id = previous_budget_id
        litellm.validate_end_user_id_in_db = previous_validate
    end_user_id = call.get("end_user_id")
    stored = None if end_user_id is None else stored_view(
        fixture.values.get(end_user_cache_key(end_user_id))
    )
    return Answer(
        value=value,
        stored=stored,
        raised=raised,
        reads=list(fixture.reads),
        writes=list(fixture.writes),
        queries=list(fixture.queries),
    )


def run(input_value: Call) -> Answer:
    parsed = TypeAdapter(Call).validate_python(input_value)
    return asyncio.run(answer(parsed))
