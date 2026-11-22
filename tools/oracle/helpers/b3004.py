import asyncio
from collections.abc import Mapping, Sequence
from typing import Final, NotRequired, Protocol, TypedDict

import litellm
from fastapi import HTTPException
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.exceptions import BudgetExceededError
from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_TeamTable, LiteLLM_UserTable, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.spend_tracking import budget_reservation
from litellm.proxy.spend_tracking.spend_counter_batch import PendingSpendIncrement
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ROUTE: Final[str] = "/v1/chat/completions"
REQUEST_BODY: Final[dict[str, JsonValue]] = {"model": "gpt-4o"}
# captured before the case scripts it, so the recorder still applies the real increments
REAL_PIPELINE: Final = proxy_server.run_spend_counter_pipeline
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class BudgetLimit(TypedDict):
    budget_duration: str
    max_budget: float


class TokenInput(TypedDict):
    token: str
    max_budget: float | None
    spend: float
    tpm_limit: int | None
    rpm_limit: int | None
    throttle: bool
    user_id: str
    budget_limits: NotRequired[list[BudgetLimit]]


class UserInput(TypedDict):
    user_id: str
    max_budget: float
    spend: float


class Input(TypedDict):
    token: TokenInput
    reservation_cost: float
    fail_closed: bool
    throttle_percentage: float | None
    user: NotRequired[UserInput]
    memory: NotRequired[dict[str, float]]
    sources: NotRequired[dict[str, float]]
    failures: NotRequired[list[str]]
    pipeline_fails: NotRequired[bool]


class Entry(TypedDict):
    counter_key: str
    reserved_cost: float
    applied_adjustment: float


class Reservation(TypedDict):
    reserved_cost: float
    entries: list[Entry]


RESERVATION: Final[TypeAdapter[Reservation]] = TypeAdapter(Reservation)


# reserve_budget_for_request as the case calls it: the same reservation entry point, its answer
# read as a JSON object so no untyped LiteLLM dict enters the helper
class BudgetReservation(Protocol):
    async def reserve_budget_for_request(
        self,
        request_body: dict[str, JsonValue],
        route: str,
        llm_router: Router | None,
        valid_token: UserAPIKeyAuth | None,
        team_object: LiteLLM_TeamTable | None,
        user_object: LiteLLM_UserTable | None,
        prisma_client: PrismaClient | None,
        user_api_key_cache: UserApiKeyCache,
        proxy_logging_obj: ProxyLogging,
        end_user_id: str | None = None,
        end_user_object: object = None,
        apply_user_budget_to_team_keys: bool = False,
        fail_closed_budget_enforcement: bool = False,
        raw_body: bytes | None = None,
    ) -> dict[str, JsonValue] | None: ...


class ScriptedMemory(InMemoryCache):
    def __init__(self, values: dict[str, float], failures: frozenset[str]) -> None:
        super().__init__()
        self.values = dict(values)
        self.failures = failures
        self.deleted: list[str] = []

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> float | None:
        del kwargs
        if key in self.failures:
            raise ValueError(f"the scripted spend counter {key} is unavailable")
        return self.values.get(key)

    @override
    def set_cache(self, key: str, value: float, **kwargs: float | None) -> None:
        del kwargs
        self.values[key] = value

    @override
    def delete_cache(self, key: str) -> None:
        self.deleted.append(key)
        self.values.pop(key, None)


class ScriptedSources(InMemoryCache):
    # the source cache holds the auth row's JSON payload, which _get_source_cache_base_spend reads "spend" from
    def __init__(self, sources: dict[str, float]) -> None:
        super().__init__()
        self.entries: dict[str, dict[str, float]] = {}
        for key, amount in sources.items():
            self.entries[key] = {"spend": amount}

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> dict[str, float] | None:
        del kwargs
        return self.entries.get(key)


class Pipeline:
    def __init__(self, *, fail: bool) -> None:
        self.fail = fail
        self.calls: list[JsonValue] = []

    async def __call__(self, pending: Sequence[PendingSpendIncrement]) -> tuple[float | None, ...]:
        if not pending:
            return ()
        recorded: list[JsonValue] = []
        for item in pending:
            pair: list[JsonValue] = [item.counter_key, item.increment]
            recorded.append(pair)
        self.calls.append(recorded)
        if self.fail:
            raise RuntimeError("the scripted spend counter pipeline failed")
        return await REAL_PIPELINE(pending=pending)


class MaxCost:
    def __init__(self, cost: float) -> None:
        self.cost = cost

    def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None:
        del request_body, route, llm_router, input_token_counts
        return self.cost


class InputCost:
    def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None:
        del request_body, route, llm_router, input_token_counts
        return 0.0


class InputTokens:
    async def __call__(
        self,
        request_body: Mapping[str, JsonValue],
        route: str,
        llm_router: Router | None,
        raw_body: bytes | None = None,
    ) -> Mapping[str, int]:
        del request_body, route, llm_router, raw_body
        return {}


def token_object(data: TokenInput) -> UserAPIKeyAuth:
    limits: list[dict[str, JsonValue]] | None = None
    budget_limits = data.get("budget_limits")
    if budget_limits is not None:
        limits = [
            {"budget_duration": limit["budget_duration"], "max_budget": limit["max_budget"]}
            for limit in budget_limits
        ]
    metadata: dict[str, JsonValue] = {"throttle_on_budget_exceeded": data["throttle"]}
    return UserAPIKeyAuth(
        token=data["token"],
        max_budget=data["max_budget"],
        spend=data["spend"],
        tpm_limit=data["tpm_limit"],
        rpm_limit=data["rpm_limit"],
        user_id=data["user_id"],
        budget_limits=limits,
        metadata=metadata,
    )


def user_object(data: UserInput | None) -> LiteLLM_UserTable | None:
    if data is None:
        return None
    return LiteLLM_UserTable(
        user_id=data["user_id"], max_budget=data["max_budget"], spend=data["spend"]
    )


def entry_json(entry: Entry) -> JsonValue:
    described: dict[str, JsonValue] = {
        "counter_key": entry["counter_key"],
        "reserved_cost": entry["reserved_cost"],
        "applied_adjustment": entry["applied_adjustment"],
    }
    return described


def reservation_json(reservation: Mapping[str, JsonValue] | None) -> JsonValue:
    if reservation is None:
        return None
    parsed = RESERVATION.validate_python(reservation)
    entries: list[JsonValue] = []
    for entry in parsed["entries"]:
        entries.append(entry_json(entry))
    described: dict[str, JsonValue] = {
        "reserved_cost": parsed["reserved_cost"],
        "entries": entries,
    }
    return described


def error_json(error: BudgetExceededError | HTTPException) -> JsonValue:
    described: dict[str, JsonValue] = {
        "class": type(error).__name__,
        "status": error.status_code,
    }
    return described


def memory_json(memory: ScriptedMemory) -> JsonValue:
    described: dict[str, JsonValue] = {}
    for key in sorted(memory.values):
        described[key] = memory.values[key]
    return described


def invalidated_json(memory: ScriptedMemory) -> JsonValue:
    listed: list[JsonValue] = []
    for key in memory.deleted:
        listed.append(key)
    return listed


async def execute(data: Input) -> JsonValue:
    values = data.get("memory")
    if values is None:
        values = {}
    failures = data.get("failures")
    if failures is None:
        failures = []
    sources = data.get("sources")
    if sources is None:
        sources = {}
    pipeline_fails = data.get("pipeline_fails")
    if pipeline_fails is None:
        pipeline_fails = False
    token = token_object(data["token"])
    user = user_object(data.get("user"))
    memory = ScriptedMemory(values, frozenset(failures))
    source_cache = UserApiKeyCache(in_memory_cache=ScriptedSources(sources))
    pipeline = Pipeline(fail=pipeline_fails)
    # each helper call runs in its own process, so the proxy globals get set in place
    proxy_server.spend_counter_cache.in_memory_cache = memory
    proxy_server.spend_counter_cache.redis_cache = None
    proxy_server.prisma_client = None
    proxy_server.user_api_key_cache = source_cache
    settings: dict[str, JsonValue] = {}
    proxy_server.general_settings = settings
    litellm.budget_exceeded_throttle_percentage = data["throttle_percentage"]
    budget_reservation.estimate_request_max_cost = MaxCost(data["reservation_cost"])
    budget_reservation.estimate_request_input_cost = InputCost()
    budget_reservation.count_request_input_tokens = InputTokens()
    proxy_server.run_spend_counter_pipeline = pipeline
    # the module object is the entry point the case exercises, read through the protocol
    budget: BudgetReservation = budget_reservation
    reservation: dict[str, JsonValue] | None = None
    failure: JsonValue = None
    try:
        reservation = await budget.reserve_budget_for_request(
            request_body=REQUEST_BODY,
            route=ROUTE,
            llm_router=None,
            valid_token=token,
            team_object=None,
            user_object=user,
            prisma_client=None,
            user_api_key_cache=source_cache,
            proxy_logging_obj=proxy_server.proxy_logging_obj,
            fail_closed_budget_enforcement=data["fail_closed"],
        )
    except (BudgetExceededError, HTTPException) as error:
        failure = error_json(error)
    output: dict[str, JsonValue] = {
        "reservation": reservation_json(reservation),
        "error": failure,
        "memory": memory_json(memory),
        "pipelines": pipeline.calls,
        "invalidated": invalidated_json(memory),
    }
    return JSON.validate_python(output)


def run(input_value: Input) -> JsonValue:
    return asyncio.run(execute(input_value))
