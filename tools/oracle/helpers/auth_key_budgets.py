import asyncio
import sys
from datetime import datetime, tzinfo
from types import ModuleType
from typing import ClassVar, Final, Literal, NotRequired, Self, TypedDict

import litellm
from litellm import constants
from litellm.exceptions import BudgetExceededError
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import CallInfo, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks, user_api_key_auth
from litellm.proxy.utils import ProxyLogging
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
PIN: Final = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Function = Literal["admit", "temp_update", "email_merge", "team_member"]
GlobalMap = dict[str, list[JsonValue]]
EmailMap = dict[str, str | list[JsonValue] | None]


class Call(TypedDict):
    function: Function
    token: NotRequired[dict[str, JsonValue] | None]
    spends: NotRequired[list[float] | None]
    now: NotRequired[str | None]
    throttle_pct: NotRequired[float | None]
    global_cfg: NotRequired[GlobalMap | None]
    user_email: NotRequired[str | None]
    per_key_cfg: NotRequired[EmailMap | None]
    team_id: NotRequired[str | None]
    team_alias: NotRequired[str | None]
    team_metadata: NotRequired[dict[str, JsonValue] | None]
    organization_id: NotRequired[str | None]
    user_id: NotRequired[str | None]
    spend: NotRequired[float | None]
    max_budget: NotRequired[float | None]


class Request(TypedDict):
    calls: list[Call]


class FixedClock(datetime):
    moment: ClassVar[datetime | None] = None

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = cls.moment
        if moment is None:
            raise RuntimeError("the clock is not fixed")
        return cls.fromtimestamp(moment.timestamp(), tz)


class Recorder(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: Final[list[JsonValue]] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        self.alerts.append(
            {"type": type, "info": JSON.validate_json(user_info.model_dump_json())}
        )


class ScriptedSpend:
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
        start, offset = start_fields(window_start)
        self.reads.append(
            {
                "counter_key": counter_key,
                "fallback_spend": fallback_spend,
                "max_budget": max_budget,
                "entity_type": window_entity_type,
                "entity_id": window_entity_id,
                "duration": window_duration,
                "start": start,
                "offset_seconds": offset,
            }
        )
        index = len(self.reads) - 1
        if index >= len(self.spends):
            raise RuntimeError("the scripted spend ran out")
        return self.spends[index]


class ProxyServer(ModuleType):
    get_current_spend: ScriptedSpend


def start_fields(start: datetime | None) -> tuple[str | None, int | None]:
    if start is None:
        return None, None
    offset = start.utcoffset()
    text = start.replace(tzinfo=None).isoformat()
    if offset is None:
        return text, None
    return text, int(offset.total_seconds())


def auth_of(call: Call) -> UserAPIKeyAuth:
    token = call.get("token")
    if token is None:
        raise ValueError("the call names the token fields")
    return UserAPIKeyAuth.model_validate(token)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def owner(call: Call) -> LiteLLM_UserTable | None:
    email = call.get("user_email")
    if email is None:
        return None
    return LiteLLM_UserTable(user_id="key-owner", user_email=email)


def fix_clock(call: Call) -> None:
    moment = call.get("now")
    if moment is None:
        raise ValueError("the call names the fixed clock")
    FixedClock.moment = datetime.fromisoformat(moment)
    user_api_key_auth.datetime = FixedClock  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L3615-L3623


def denial_record(denied: BudgetExceededError) -> JsonValue:
    return {
        "status": denied.status_code,
        "current_cost": denied.current_cost,
        "max_budget": denied.max_budget,
        "entity_type": denied.entity_type,
        "entity_id": denied.entity_id,
    }


async def admit_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    spends = call.get("spends")
    if spends is None:
        raise ValueError("the call scripts the spend reads")
    fix_clock(call)
    recorder = Recorder()
    reader = ScriptedSpend(spends)
    proxy = ProxyServer(PROXY_MODULE)
    proxy.get_current_spend = reader
    sys.modules[PROXY_MODULE] = proxy
    litellm.budget_exceeded_throttle_percentage = call.get("throttle_pct")
    litellm.default_key_max_budget_alert_emails = call.get("global_cfg")
    user = owner(call)
    updated = user_api_key_auth._update_key_budget_with_temp_budget_increase(fixed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L3626-L3634
    denial: JsonValue = None
    try:
        await auth_checks._virtual_key_max_budget_alert_check(updated, recorder, user)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5683-L5762
        await auth_checks._virtual_key_max_budget_check(updated, recorder, user)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5479-L5556
        await auth_checks._virtual_key_soft_budget_check(updated, recorder, user)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5603-L5639
        await auth_checks._virtual_key_multi_budget_check(updated)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5559-L5600
    except BudgetExceededError as denied:
        denial = denial_record(denied)
    # a scheduled alert task only runs once the request path yields
    await asyncio.sleep(0)
    return {
        "alerts": list(recorder.alerts),
        "reads": list(reader.reads),
        "throttle_pct": updated.budget_throttle_pct,
        "fraction": constants.EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE,
        "denial": denial,
    }


def temp_update_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    fix_clock(call)
    updated = user_api_key_auth._update_key_budget_with_temp_budget_increase(fixed)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L3626-L3634
    return {"max_budget": updated.max_budget}


def email_merge_call(call: Call) -> JsonValue:
    merged = auth_checks._merge_budget_alert_email_configs(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5664-L5680
        global_cfg=call.get("global_cfg"), per_key_cfg=call.get("per_key_cfg")
    )
    return {"merged": JSON.validate_python(merged)}


async def team_member_call(call: Call) -> JsonValue:
    team_id = call.get("team_id")
    user_id = call.get("user_id")
    spend = call.get("spend")
    max_budget = call.get("max_budget")
    if team_id is None or user_id is None or spend is None or max_budget is None:
        raise ValueError("a team member alert names team_id, user_id, spend and max_budget")
    recorder = Recorder()
    auth_checks._team_member_max_budget_alert_check(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5789-L5820
        team_id=team_id,
        team_alias=call.get("team_alias"),
        team_metadata=call.get("team_metadata"),
        organization_id=call.get("organization_id"),
        user_id=user_id,
        user_email=call.get("user_email"),
        proxy_logging_obj=recorder,
        spend=spend,
        max_budget=max_budget,
    )
    await asyncio.sleep(0)
    return {"alerts": list(recorder.alerts)}


async def call_answer(call: Call) -> JsonValue:
    try:
        return await answer_for(call)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L5479-L5556
        return {"ok": False}


async def answer_for(call: Call) -> JsonValue:
    match call["function"]:
        case "admit":
            return {"ok": True, "value": await admit_call(call)}
        case "temp_update":
            return {"ok": True, "value": temp_update_call(call)}
        case "email_merge":
            return {"ok": True, "value": email_merge_call(call)}
        case "team_member":
            return {"ok": True, "value": await team_member_call(call)}


async def answer(parsed: Request) -> JsonValue:
    return [
        await call_answer(call)
        for call in parsed["calls"]
    ]


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = TypeAdapter(Request).validate_python(input_value)
    return asyncio.run(answer(parsed))
