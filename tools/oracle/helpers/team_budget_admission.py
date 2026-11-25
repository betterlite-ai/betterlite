import asyncio
import logging
import sys
import time
from collections.abc import Awaitable
from datetime import datetime, tzinfo
from functools import partial
from types import ModuleType
from typing import ClassVar, Final, Literal, NotRequired, Self, TypedDict

from litellm.exceptions import BudgetExceededError
from litellm.models.team import LiteLLM_TeamTable
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import CallInfo, UserAPIKeyAuth
from litellm.proxy._types import ModelAccessDeniedProxyException as ModelDenied
from litellm.proxy.auth import auth_checks
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.spend_tracking import budget_reservation
from litellm.proxy.utils import PrismaClient, ProxyLogging
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SOFT_CHECK: Final = "_team_soft_budget_check"


class Call(TypedDict):
    function: Literal["admit", "soft"]
    team: NotRequired[dict[str, JsonValue] | None]
    token: NotRequired[dict[str, JsonValue] | None]
    membership: NotRequired[dict[str, JsonValue] | None]
    default_budget: NotRequired[dict[str, JsonValue] | None]
    user_email: NotRequired[str | None]
    model: NotRequired[str | list[str] | None]
    spends: list[float]
    now: str


class Request(TypedDict):
    calls: list[Call]


class Clock:
    moment: ClassVar[datetime | None] = None


def fixed_now() -> datetime:
    moment = Clock.moment
    if moment is None:
        raise RuntimeError("the clock is not fixed")
    return moment


# budget_reservation's isinstance must keep admitting pydantic's plain datetimes
class _FrozenMeta(type):
    @override
    def __instancecheck__(self, instance: JsonValue | datetime) -> bool:
        return isinstance(instance, datetime)  # ast-grep-ignore: no-isinstance


class FrozenDatetime(datetime, metaclass=_FrozenMeta):
    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(fixed_now().timestamp(), tz)


class Recorder(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: Final[list[JsonValue]] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        self.alerts.append({"type": type, "info": JSON.validate_json(user_info.model_dump_json())})


class SoftLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.messages: Final[list[str]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.funcName == SOFT_CHECK:
            self.messages.append(record.getMessage())


class CounterReads:
    def __init__(self, spends: list[float]) -> None:
        self.spends: Final = spends
        self.reads: Final[list[JsonValue]] = []

    async def __call__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2797-L2806
        self,
        counter_key: str,
        fallback_spend: float,
        max_budget: float | None = None,
        window_entity_type: str | None = None,
        window_entity_id: str | None = None,
        window_duration: str | None = None,
        window_start: datetime | None = None,
        *,
        fallback_authoritative: bool = False,
    ) -> float:
        del fallback_authoritative
        self.reads.append(
            {
                "counter_key": counter_key,
                "fallback_spend": fallback_spend,
                "max_budget": max_budget,
                "entity_type": window_entity_type,
                "entity_id": window_entity_id,
                "duration": window_duration,
                "start": None if window_start is None else window_start.isoformat(),
            }
        )
        if len(self.reads) > len(self.spends):
            raise RuntimeError("the scripted spend ran out")
        return self.spends[len(self.reads) - 1]


# log_db_metrics reports the default budget lookup to the proxy's service logger
class ServiceLog:
    def __init__(self) -> None:
        self.service_logging_obj: Final = self

    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...

    async def async_service_failure_hook(self, **_kwargs: JsonValue) -> None: ...


class ProxyServer(ModuleType):
    get_current_spend: CounterReads
    proxy_logging_obj: ServiceLog


# the default budget comes from the seeded cache, so the client only says a database is configured
class SeededDatabase(PrismaClient):
    def __init__(self) -> None:
        pass


def team_of(call: Call) -> LiteLLM_TeamTable | None:
    team = call.get("team")
    return None if team is None else LiteLLM_TeamTable.model_validate(team)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/team.py#L86


def token_of(call: Call) -> UserAPIKeyAuth | None:
    token = call.get("token")
    return None if token is None else UserAPIKeyAuth.model_validate(token)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def membership_of(call: Call) -> LiteLLM_TeamMembership | None:
    membership = call.get("membership")
    return None if membership is None else LiteLLM_TeamMembership.model_validate(membership)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/team_membership.py#L12


def owner(call: Call) -> LiteLLM_UserTable | None:
    email = call.get("user_email")
    return None if email is None else LiteLLM_UserTable(user_id="key-owner", user_email=email)


def denial_record(denied: BudgetExceededError) -> JsonValue:
    return {
        "status": denied.status_code,
        "current_cost": denied.current_cost,
        "max_budget": denied.max_budget,
        "entity_type": denied.entity_type,
        "entity_id": denied.entity_id,
    }


async def seeded_cache(call: Call) -> UserApiKeyCache:
    cache = UserApiKeyCache()
    default_budget = call.get("default_budget")
    if default_budget is not None:
        cache_key = f"team_member_default_budget:{default_budget.get('budget_id')}"
        await cache.async_set_cache(key=cache_key, value=default_budget)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/user_api_key_cache.py#L215
    return cache


async def settled(check: Awaitable[None]) -> BudgetExceededError | None:
    try:
        await check
    except BudgetExceededError as denied:
        return denied
    return None


async def admit_call(call: Call, recorder: Recorder) -> JsonValue:
    team = team_of(call)
    token = token_of(call)
    membership = membership_of(call)
    cache = await seeded_cache(call)
    database = SeededDatabase()
    reader = CounterReads(call["spends"])
    proxy = ProxyServer(PROXY_MODULE)
    proxy.get_current_spend = reader
    proxy.proxy_logging_obj = ServiceLog()
    sys.modules[PROXY_MODULE] = proxy
    model = call.get("model")
    # common_checks asks for the member scope only with a model, a team and a token user
    try:
        if model and team is not None and token is not None and token.user_id is not None:
            await auth_checks._check_team_member_model_access(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5913-L5972
                model=model,
                team_object=team,
                valid_token=token,
                llm_router=None,
                prisma_client=database,
                user_api_key_cache=cache,
                proxy_logging_obj=recorder,
                team_membership=membership,
                team_membership_loaded=True,
                key_model_aliases=auth_checks.key_model_aliases_for_auth_check(token),
            )
    except ModelDenied as denied:
        scope: JsonValue = {"type": denied.type, "param": denied.param, "code": denied.code}
        return {"reads": [], "alerts": [], "denial": None, "model_denial": scope}
    # common_checks gathers the scopes, so all settle and the first error in this order wins
    results = [
        await settled(auth_checks._team_max_budget_check(team, token, recorder)),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5975-L6022
        await settled(auth_checks._team_multi_budget_check(team)),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6025-L6062
        await settled(auth_checks._team_soft_budget_check(team, token, recorder)),  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6065-L6133
        await settled(
            auth_checks._check_team_member_budget(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5823-L5910
                team_object=team,
                user_object=owner(call),
                valid_token=token,
                prisma_client=database,
                user_api_key_cache=cache,
                proxy_logging_obj=recorder,
                team_membership=membership,
                team_membership_loaded=True,
            )
        ),
    ]
    denial = next((denial_record(result) for result in results if result is not None), None)
    # a scheduled alert task only runs once the request path yields
    await asyncio.sleep(0)
    return {
        "reads": list(reader.reads),
        "alerts": list(recorder.alerts),
        "denial": denial,
        "model_denial": None,
    }


async def soft_call(call: Call, recorder: Recorder) -> JsonValue:
    handler = SoftLog()
    proxy_logger = logging.getLogger("LiteLLM Proxy")
    previous_level = proxy_logger.level
    proxy_logger.setLevel(logging.DEBUG)
    proxy_logger.addHandler(handler)
    try:
        await auth_checks._team_soft_budget_check(team_of(call), token_of(call), recorder)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6065-L6133
    finally:
        proxy_logger.removeHandler(handler)
        proxy_logger.setLevel(previous_level)
    await asyncio.sleep(0)
    return {"alerts": list(recorder.alerts), "logs": list(handler.messages)}


async def answer_for(call: Call) -> JsonValue:
    Clock.moment = datetime.fromisoformat(call["now"])
    auth_checks.get_utc_datetime = fixed_now  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5854
    budget_reservation.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/budget_reservation.py#L1280
    time.time = partial(float, fixed_now().timestamp())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/duration_parser.py#L77
    recorder = Recorder()
    match call["function"]:
        case "admit":
            return await admit_call(call, recorder)
        case "soft":
            return await soft_call(call, recorder)


async def answer_calls(calls: list[Call]) -> JsonValue:
    answers: list[JsonValue] = []
    for call in calls:
        try:
            answers.append({"ok": True, "value": await answer_for(call)})
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5823-L5910
            answers.append({"ok": False})
    return answers


def run(input_value: Request) -> JsonValue:
    calls = input_value["calls"]
    return asyncio.run(answer_calls(calls))
