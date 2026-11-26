import asyncio
from collections.abc import Mapping
from dataclasses import dataclass, field
from typing import Final, Literal, Required, TypedDict, cast

import litellm
from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import CallInfo, LiteLLM_UserTable, UserAPIKeyAuth
from litellm.proxy.auth import user_api_key_auth
from litellm.proxy.common_utils import http_parsing_utils
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, ProxyLogging
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from typing_extensions import override
from auth_model_budgets.end_user import auth_of, end_user_apply_call, end_user_update_call

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Call(TypedDict, total=False):
    function: Required[Literal[
        "key_fallback", "user_budget", "team_budget",
        "end_user_apply", "end_user_update", "global_spend",
    ]]
    token: dict[str, JsonValue] | None
    models: list[str] | None
    model_name: str | None
    request_data: dict[str, JsonValue] | None
    path_params: dict[str, JsonValue] | None
    failing: bool | None
    fallback: str | None
    failing_models: list[str] | None
    budget: dict[str, JsonValue] | None
    params: dict[str, JsonValue] | None
    end_user_id: str | None
    max_budget: float | None
    spend: float | None
    database: bool | None
    admin_name: str | None
    tokens: list[str] | None


def verdict(*, failing: bool, entity_type: str, entity_id: str) -> bool:
    if not failing:
        return True
    raise BudgetExceededError(current_cost=12.5, max_budget=10.0, message="Budget has been exceeded!", llm_provider="openai", entity_type=entity_type, entity_id=entity_id)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L998-L1010


def denial_record(denied: BudgetExceededError) -> JsonValue:
    return {"exception": type(denied).__name__, "status": denied.status_code, "current_cost": denied.current_cost, "max_budget": denied.max_budget, "entity_type": denied.entity_type, "entity_id": denied.entity_id}  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L998-L1010


def views(request: Request) -> tuple[JsonValue, JsonValue, JsonValue]:
    parsed = cast("dict[str, JsonValue] | None", http_parsing_utils._safe_get_request_parsed_body(request=request))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L303
    cached = cast("dict[str, JsonValue]", request._json)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L484
    return (None if parsed is None else parsed.get("model"), cached.get("model"), OBJECT.validate_json(request._body).get("model"))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L483-L485


@dataclass
class ScriptedLimiter:
    failing: bool = False
    fallback: str | None = None
    failing_models: list[str] = field(default_factory=list)
    calls: list[str] = field(default_factory=list)
    seen: list[JsonValue] = field(default_factory=list)

    async def is_key_within_model_budget(self, user_api_key_dict: UserAPIKeyAuth, model: str) -> bool:  # noqa: ARG002, E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L450-L453
        self.calls.append(f"key:{model}")
        return verdict(failing=self.failing, entity_type="key", entity_id="hashed-key")  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L450-L453

    async def get_fallback_model_within_budget(  # ast-grep-ignore: no-long-names
        self, user_api_key_dict: UserAPIKeyAuth, model: str
    ) -> str | None:
        del user_api_key_dict
        self.calls.append(f"fallback:{model}")
        return self.fallback

    async def is_user_within_model_budget(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L391-L395
        self,
        user_id: str,
        user_model_max_budget: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        model: str,
    ) -> bool:
        self.seen.append({"user_id": user_id, "model": model, "budget": OBJECT.validate_python(user_model_max_budget)})  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L391-L395
        return verdict(failing=model in self.failing_models, entity_type="user", entity_id=user_id)

    async def is_team_within_model_budget(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L409-L414
        self,
        team_id: str,
        team_model_max_budget: Mapping[str, object],  # ast-grep-ignore: no-loose-types
        key_model_max_budget: Mapping[str, object] | None,  # ast-grep-ignore: no-loose-types
        model: str,
    ) -> bool:
        key: JsonValue = None if key_model_max_budget is None else OBJECT.validate_python(key_model_max_budget)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L407
        record: JsonValue = {"team_id": team_id, "model": model, "team_budget": OBJECT.validate_python(team_model_max_budget), "key_budget": key}  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L409-L414
        self.seen.append(record)
        return verdict(failing=model in self.failing_models, entity_type="team", entity_id=team_id)


@dataclass
class SpendTable:
    spend: float | None
    gated: bool
    reads: list[str] = field(default_factory=list)
    started: asyncio.Event = field(default_factory=asyncio.Event)
    release: asyncio.Event = field(default_factory=asyncio.Event)

    async def find_unique(self, *, where: dict[str, str]) -> LiteLLM_UserTable | None:
        del where
        self.reads.append("proxy_budget")
        self.started.set()
        if self.gated:
            await self.release.wait()
        return None if self.spend is None else LiteLLM_UserTable(user_id="litellm-proxy-budget", spend=self.spend)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L733-L757


class SpendDatabase(PrismaWrapper):
    def __init__(self, table: SpendTable) -> None: self.litellm_usertable = table


class SpendPrisma(PrismaClient):
    def __init__(self, table: SpendTable) -> None: self.db = SpendDatabase(table)


class AlertRecorder(ProxyLogging):
    def __init__(self) -> None: self.alerts: list[JsonValue] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        dumped = OBJECT.validate_json(user_info.model_dump_json())
        info = {name: dumped.get(name) for name in ("user_id", "max_budget", "spend", "token", "event_group")}  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L3046-L3061
        self.alerts.append({"type": type, "info": info})


@dataclass
class SpendRun:
    admin: str
    cache: UserApiKeyCache
    prisma: SpendPrisma | None
    recorder: AlertRecorder

    async def spend(self, token: str) -> float | None:
        return await user_api_key_auth.get_global_proxy_spend(self.admin, self.cache, self.prisma, token, self.recorder)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L760-L790

    async def follower(self, table: SpendTable, token: str) -> float | None:
        await table.started.wait()
        table.release.set()
        return await self.spend(token)


async def key_fallback_call(call: Call) -> JsonValue:
    fixed = auth_of(call)
    model_name = call.get("model_name")
    request_data = call.get("request_data")
    if model_name is None or request_data is None:
        raise ValueError("a key fallback call names the model and the request data")
    limiter = ScriptedLimiter(failing=bool(call.get("failing")), fallback=call.get("fallback"))
    params = call.get("path_params")
    scope: dict[str, JsonValue] = {"type": "http", "path_params": params}
    request = Request(scope)
    request._json = dict[str, JsonValue]()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L484
    request._body = b"{}"  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L485
    error: JsonValue = None
    try:
        await user_api_key_auth._check_key_model_budget_with_fallback(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L417-L488
            valid_token=fixed,
            model_max_budget_limiter=limiter,
            model_name=model_name,
            request_data=request_data,
            request=request,
        )
    except BudgetExceededError as denied:
        error = denial_record(denied)
    parsed, cached, body = views(request)
    return {
        "calls": list(limiter.calls),
        "request_model": request_data.get("model"),
        "parsed_model": parsed,
        "json_model": cached,
        "body_model": body,
        "path_model": None if params is None else params.get("model"),
        "error": error,
    }


async def budget_call(call: Call, entity: Literal["user", "team"]) -> JsonValue:
    fixed = auth_of(call)
    models = call.get("models")
    if models is None:
        raise ValueError("a budget call names the models")
    failing_models = call.get("failing_models")
    limiter = ScriptedLimiter(failing_models=[] if failing_models is None else failing_models)
    if entity == "user":
        work = user_api_key_auth._check_user_model_budget(fixed, limiter, models)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L377-L395
    else:
        work = user_api_key_auth._check_team_model_budget(fixed, limiter, models)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L398-L414
    error: JsonValue = None
    try:
        await work
    except BudgetExceededError as denied:
        error = denial_record(denied)
    return {"calls": list(limiter.seen), "error": error}


async def global_spend_call(call: Call) -> JsonValue:
    max_budget = call.get("max_budget")
    tokens = call.get("tokens")
    admin = call.get("admin_name")
    if max_budget is None or not tokens or admin is None:
        raise ValueError("a global spend call names max_budget, tokens and the admin name")
    litellm.max_budget = max_budget
    table = SpendTable(call.get("spend"), len(tokens) > 1)
    prisma = None if call.get("database") is False else SpendPrisma(table)
    spend = SpendRun(admin, UserApiKeyCache(), prisma, AlertRecorder())
    first: float | None
    second: float | None = None
    if len(tokens) > 1:
        first, second = await asyncio.gather(spend.spend(tokens[0]), spend.follower(table, tokens[1]))  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L733-L757
    else:
        first = await spend.spend(tokens[0])
    await asyncio.sleep(0)
    return {"spend": first, "second": second, "alerts": list(spend.recorder.alerts), "reads": len(table.reads)}  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L760-L790


async def answer_for(call: Call) -> JsonValue:
    match call["function"]:
        case "key_fallback":
            return await key_fallback_call(call)
        case "user_budget":
            return await budget_call(call, "user")
        case "team_budget":
            return await budget_call(call, "team")
        case "end_user_apply":
            return end_user_apply_call(call)
        case "end_user_update":
            return end_user_update_call(call)
        case "global_spend":
            return await global_spend_call(call)


async def answer_calls(calls: list[Call]) -> JsonValue:
    answers: list[JsonValue] = []
    for call in calls:
        try:
            answers.append({"ok": True, "value": await answer_for(call)})
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L417-L488
            answers.append({"ok": False})
    return answers


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    calls = TypeAdapter(list[Call]).validate_python(input_value["calls"])
    return asyncio.run(answer_calls(calls))
