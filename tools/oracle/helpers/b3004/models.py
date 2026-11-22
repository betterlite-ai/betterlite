from collections.abc import Awaitable, Callable
from typing import Final, NotRequired, Protocol, TypedDict, runtime_checkable

from litellm.proxy._types import LiteLLM_TeamTable, LiteLLM_UserTable, UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter


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
class BudgetReservation(Protocol):
    reserve_budget_for_request: ReserveBudget
