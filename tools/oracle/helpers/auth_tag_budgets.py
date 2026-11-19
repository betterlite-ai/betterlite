import asyncio
from typing import Final, Literal, NotRequired, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.tag import LiteLLM_TagTable
from litellm.proxy.auth import auth_checks
from litellm.proxy.auth.auth_checks import (
    get_model_access_group_budgets_batch,
    get_tag_objects_batch,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    UserApiKeyCache,
    model_access_group_cache_key,
    tag_cache_key,
)
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.types.proxy.model_access_group_budget import ModelAccessGroupBudget
from pydantic import JsonValue
from typing_extensions import override

MANAGEMENT_TTL_SECONDS: Final = 60.0


class BudgetFields(TypedDict):
    max_budget: NotRequired[float | None]


class GroupFields(TypedDict):
    access_group_name: str
    spend: NotRequired[float | None]
    litellm_budget_table: NotRequired[BudgetFields | None]


class GeneratedFixture(TypedDict):
    prefix: str
    count: int


class CallBase(TypedDict):
    names: list[str]
    database: bool
    generated: NotRequired[GeneratedFixture]
    fail_query: bool


class TagCall(CallBase):
    function: Literal["tags", "tag_registry"]
    cached: list[dict[str, JsonValue]]
    rows: list[dict[str, JsonValue]]


class GroupCall(CallBase):
    function: Literal["groups", "group_registry"]
    cached: list[dict[str, JsonValue]]
    rows: list[GroupFields]


Input = TagCall | GroupCall


class TagView(TypedDict):
    models: list[str]
    spend: float
    max_budget: float | None


class GroupView(TypedDict):
    spend: float
    max_budget: float | None


class Answer(TypedDict):
    tags: dict[str, TagView]
    groups: dict[str, GroupView]
    registry: list[str] | None
    reads: list[str]
    writes: list[tuple[str, float]]
    queries: list[list[str]]
    takes: list[int]


Cached = JsonValue | LiteLLM_TagTable | ModelAccessGroupBudget | tuple[str, ...]


def run(input_value: Input) -> Answer:
    match input_value["function"]:
        case "tags":
            return asyncio.run(tag_answer(input_value))
        case "tag_registry":
            return asyncio.run(tag_answer(input_value))
        case "groups":
            return asyncio.run(group_answer(input_value))
        case "group_registry":
            return asyncio.run(group_answer(input_value))


class Fixture(InMemoryCache):
    def __init__(self) -> None:
        super().__init__()
        self.values: dict[str, Cached] = {}
        self.reads: list[str] = []
        self.writes: list[tuple[str, float]] = []
        self.queries: list[list[str]] = []
        self.takes: list[int] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> Cached:
        del kwargs
        self.reads.append(key)
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: Cached, **kwargs: float) -> None:
        ttl = kwargs.get("ttl")
        if ttl is None:
            raise ValueError("a management object write carries a ttl")
        self.writes.append((key, float(ttl)))
        self.values[key] = value

    def cache(self) -> UserApiKeyCache:
        return UserApiKeyCache(
            in_memory_cache=self, default_in_memory_ttl=MANAGEMENT_TTL_SECONDS
        )

    def answer(
        self,
        tags: dict[str, TagView],
        groups: dict[str, GroupView],
        registry: list[str] | None,
    ) -> Answer:
        return Answer(
            tags=tags,
            groups=groups,
            registry=registry,
            reads=list(self.reads),
            writes=list(self.writes),
            queries=list(self.queries),
            takes=list(self.takes),
        )


class FakeTagRow:
    def __init__(self, tag: LiteLLM_TagTable) -> None:
        self._tag = tag
        self.tag_name = tag.tag_name

    def dict(self) -> LiteLLM_TagTable:
        return self._tag


class FakeBudgetRow:
    def __init__(self, max_budget: float | None) -> None:
        self.max_budget = max_budget


class FakeGroupRow:
    def __init__(self, fields: GroupFields) -> None:
        self.access_group_name = fields["access_group_name"]
        self.spend = fields.get("spend")
        budget = fields.get("litellm_budget_table")
        self.litellm_budget_table = (
            None if budget is None else FakeBudgetRow(budget.get("max_budget"))
        )


class FakeStore:
    def __init__(
        self,
        rows: list[tuple[str, FakeTagRow | FakeGroupRow]],
        fixture: Fixture,
        *,
        fail_query: bool,
    ) -> None:
        self._rows = rows
        self._fixture = fixture
        self._fail_query = fail_query

    async def find_many(
        self,
        where: dict[str, dict[str, list[str]]] | None = None,
        include: dict[str, bool] | None = None,
        take: int | None = None,
    ) -> list[FakeTagRow | FakeGroupRow]:
        del include
        if take is not None:
            self._fixture.takes.append(take)
            if self._fail_query:
                raise ValueError("the database query failed")
            return [row for _, row in self._rows[:take]]
        if where is None:
            return []
        names = next(iter(where.values()))["in"]
        self._fixture.queries.append(list(names))
        if self._fail_query:
            raise ValueError("the database query failed")
        return [row for name, row in self._rows if name in names]


class FakeDb(PrismaWrapper):
    def __init__(self, stores: dict[str, FakeStore]) -> None:
        self._stores = stores

    @override
    def __getattr__(self, name: str) -> FakeStore:
        try:
            return self._stores[name]
        except KeyError as error:
            raise AttributeError(name) from error


class FakePrismaClient(PrismaClient):
    def __init__(self, stores: dict[str, FakeStore]) -> None:
        self.db = FakeDb(stores)


def tag_of(fields: dict[str, JsonValue]) -> LiteLLM_TagTable:
    return LiteLLM_TagTable.model_validate(fields)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/tag.py#L16


def tag_view(tag: LiteLLM_TagTable) -> TagView:
    budget = tag.litellm_budget_table
    return TagView(
        models=list(tag.models),
        spend=tag.spend,
        max_budget=None if budget is None else budget.max_budget,
    )


def group_view(budget: ModelAccessGroupBudget) -> GroupView:
    return GroupView(spend=budget.spend, max_budget=budget.max_budget)


def tag_rows(
    fields: list[dict[str, JsonValue]],
    generated: GeneratedFixture | None,
) -> list[tuple[str, FakeTagRow | FakeGroupRow]]:
    entries = list(fields)
    if generated is not None:
        names = [f"{generated['prefix']}-{index}" for index in range(generated["count"])]
        entries.extend({"tag_name": name, "models": [], "spend": 0.0} for name in names)
    tags = [tag_of(entry) for entry in entries]
    return [(tag.tag_name, FakeTagRow(tag)) for tag in tags]


def group_rows(
    fields: list[GroupFields],
    generated: GeneratedFixture | None,
) -> list[tuple[str, FakeTagRow | FakeGroupRow]]:
    rows: list[tuple[str, FakeTagRow | FakeGroupRow]] = [
        (entry["access_group_name"], FakeGroupRow(entry)) for entry in fields
    ]
    if generated is not None:
        names = [f"{generated['prefix']}-{index}" for index in range(generated["count"])]
        rows.extend((name, FakeGroupRow({"access_group_name": name})) for name in names)
    return rows


async def tag_answer(call: TagCall) -> Answer:
    fixture = Fixture()
    for entry in call["cached"]:
        tag = tag_of(entry)
        fixture.values[tag_cache_key(tag.tag_name)] = tag
    store = FakeStore(
        tag_rows(call["rows"], call.get("generated")), fixture, fail_query=call["fail_query"]
    )
    client = FakePrismaClient({"litellm_tagtable": store})
    if call["function"] == "tags":
        found = await get_tag_objects_batch(
            call["names"], client if call["database"] else None, fixture.cache()
        )
        return fixture.answer({name: tag_view(tag) for name, tag in found.items()}, {}, None)
    loaded = await auth_checks._load_tag_registry(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2086
        prisma_client=client, user_api_key_cache=fixture.cache()
    )
    return fixture.answer({}, {}, None if loaded is None else sorted(loaded))


async def group_answer(call: GroupCall) -> Answer:
    fixture = Fixture()
    for entry in call["cached"]:
        budget = ModelAccessGroupBudget.model_validate(entry)
        fixture.values[model_access_group_cache_key(budget.access_group_name)] = budget
    store = FakeStore(
        group_rows(call["rows"], call.get("generated")), fixture, fail_query=call["fail_query"]
    )
    client = FakePrismaClient({"litellm_modelaccessgroupbudgettable": store})
    if call["function"] == "groups":
        found = await get_model_access_group_budgets_batch(
            call["names"], client if call["database"] else None, fixture.cache()
        )
        groups = {name: group_view(budget) for name, budget in found.items()}
        return fixture.answer({}, groups, None)
    loaded = await auth_checks._load_model_access_group_registry(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L2108
        prisma_client=client, user_api_key_cache=fixture.cache()
    )
    return fixture.answer({}, {}, None if loaded is None else sorted(loaded))
