import asyncio
from collections.abc import Awaitable, Callable
from typing import Final, Never, Protocol, TypedDict, runtime_checkable

from litellm.proxy import proxy_server
from litellm.proxy._types import LiteLLM_TeamTable, LiteLLM_UserTable, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.spend_tracking import budget_reservation
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import ConfigDict, JsonValue, TypeAdapter

KEY: Final[str] = "case-key"
MAX_BUDGET: Final[float] = 100.0
REQUEST_BODY: Final[dict[str, JsonValue]] = {"model": "gpt-4o"}
APPLY_TO_TEAM_KEYS: Final[bool] = False
FAIL_CLOSED: Final[bool] = False
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY: Final[ConfigDict] = ConfigDict(arbitrary_types_allowed=True)

# reserve_budget_for_request's positional parameters, through fail_closed_budget_enforcement
ReserveBudget = Callable[
    [
        dict[str, JsonValue],
        str,
        Router | None,
        UserAPIKeyAuth | None,
        LiteLLM_TeamTable | None,
        LiteLLM_UserTable | None,
        PrismaClient | None,
        UserApiKeyCache,
        ProxyLogging,
        str | None,
        None,
        bool,
        bool,
    ],
    Awaitable[dict[str, JsonValue] | None],
]


@runtime_checkable
class ReservationApi(Protocol):
    reserve_budget_for_request: ReserveBudget


class Input(TypedDict):
    routes: list[str]


class CountingError(Exception):
    pass


# raises on the call only a billed route reaches, so the case observes whether token counting ran
class CountingTokens:
    async def __call__(
        self,
        request_body: dict[str, JsonValue],
        route: str,
        llm_router: Router | None,
        raw_body: bytes | None = None,
    ) -> Never:
        del request_body, route, llm_router, raw_body
        raise CountingError


async def counted_route(
    engine: ReservationApi,
    token: UserAPIKeyAuth,
    route: str,
) -> bool:
    try:
        _ = await engine.reserve_budget_for_request(
            REQUEST_BODY,
            route,
            None,
            token,
            None,
            None,
            None,
            UserApiKeyCache(),
            proxy_server.proxy_logging_obj,
            None,
            None,
            APPLY_TO_TEAM_KEYS,
            FAIL_CLOSED,
        )
    except CountingError:
        return True
    return False


async def execute(input_value: Input) -> dict[str, JsonValue]:
    engine: ReservationApi = TypeAdapter[ReservationApi](
        ReservationApi, config=ARBITRARY
    ).validate_python(budget_reservation)
    budget_reservation.count_request_input_tokens = CountingTokens()
    token = UserAPIKeyAuth(token=KEY, max_budget=MAX_BUDGET, spend=0.0)
    counted: list[JsonValue] = [
        await counted_route(engine, token, route) for route in input_value["routes"]
    ]
    return {"counted": counted}


def run(input_value: Input) -> JsonValue:
    output = asyncio.run(execute(input_value))
    return JSON.validate_python(output)
