import asyncio
import contextlib
import io
import sys
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.exceptions import BudgetExceededError
from litellm.proxy._types import CallInfo, LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth import auth_checks, auth_exception_handler
from litellm.proxy.utils import ProxyLogging
from litellm.router import Router
from litellm.types.router import ModelGroupInfo
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope
from typing_extensions import override

from auth_tenant_budgets import CounterReads, ProxyServer, denial_record

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
RECORD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class GroupFixture(TypedDict):
    input: NotRequired[float]
    output: NotRequired[float]


class RouterFixture(TypedDict):
    groups: NotRequired[dict[str, GroupFixture]]
    deployments: NotRequired[dict[str, list[dict[str, JsonValue]]]]
    resolved: NotRequired[dict[str, dict[str, JsonValue]]]
    costs: NotRequired[dict[str, dict[str, JsonValue]]]
    cache: NotRequired[dict[str, bool]]
    failing: NotRequired[bool]


class Call(TypedDict):
    function: Literal["zero_cost", "unpriced", "request_skip", "request_checks", "project"]
    router: RouterFixture
    model: NotRequired[str | list[str]]
    route: NotRequired[str]
    method: NotRequired[str]
    body: NotRequired[dict[str, JsonValue]]
    enforce: NotRequired[bool]
    reject: NotRequired[bool]
    max_budget: NotRequired[float]
    global_spend: NotRequired[float]
    project: NotRequired[dict[str, JsonValue]]
    token: NotRequired[dict[str, JsonValue]]
    spends: NotRequired[list[float]]


class Input(TypedDict):
    calls: list[Call]


# the router the checks read, with every collaboration the case scripts recorded for comparison
class ScriptedRouter(Router):
    def __init__(self, fixture: RouterFixture) -> None:
        groups = fixture.get("groups")
        deployments = fixture.get("deployments")
        resolved = fixture.get("resolved")
        seeded = fixture.get("cache")
        self.groups: dict[str, GroupFixture] = {} if groups is None else groups
        self.deployments: dict[str, list[dict[str, JsonValue]]] = (
            {} if deployments is None else deployments
        )
        self.resolved: dict[str, dict[str, JsonValue]] = {} if resolved is None else resolved
        self.lookups: list[str] = []
        self.cache_entries: dict[str, bool] = {} if seeded is None else dict(seeded)
        self._zero_cost_cache = self.cache_entries  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L1026
        self.failing: bool = fixture.get("failing") is True

    @override
    def get_model_group_info(self, model_group: str) -> ModelGroupInfo | None:
        self.lookups.append(f"group:{model_group}")
        self.check_available()
        group = self.groups.get(model_group)
        if group is None:
            return None
        return ModelGroupInfo(
            model_group=model_group,
            providers=list[str](),
            input_cost_per_token=group.get("input"),
            output_cost_per_token=group.get("output"),
        )

    @override
    def get_model_list(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L12041
        self, model_name: str | None = None, team_id: str | None = None
    ) -> list[dict[str, JsonValue]] | None:
        _ = team_id
        self.lookups.append(f"list:{model_name}")
        self.check_available()
        rows = None if model_name is None else self.deployments.get(model_name)
        return None if rows is None else list(rows)

    @override
    def get_deployment_model_info(  # type: ignore[override]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L10830
        self, model_id: str, model_name: str
    ) -> dict[str, JsonValue] | None:
        self.lookups.append(f"info:{model_id}|{model_name}")
        self.check_available()
        return self.resolved.get(model_id)

    def check_available(self) -> None:
        if self.failing:
            raise RuntimeError("the model catalog could not answer")


# both project checks queue their alert through budget_alerts; the case compares amounts
class ProjectAlerts(ProxyLogging):
    def __init__(self) -> None:
        self.alerts: list[JsonValue] = []

    @override
    async def budget_alerts(self, type: str, user_info: CallInfo) -> None:
        self.alerts.append(
            {
                "type": type,
                "spend": user_info.spend,
                "max_budget": user_info.max_budget,
                "soft_budget": user_info.soft_budget,
            }
        )


def route_of(call: Call) -> str:
    route = call.get("route")
    if route is None:
        raise ValueError("the call needs a route")
    return route


def http_request(method: str, route: str) -> Request:
    scope: Scope = {"type": "http", "method": method, "path": route}
    return Request(scope=scope)


def envelope_of(error: Exception) -> JsonValue:
    converted = auth_exception_handler._as_proxy_exception(error)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
    return {
        "type": converted.type,
        "param": converted.param,
        "code": converted.code,
    }


def observed(router: ScriptedRouter) -> dict[str, JsonValue]:
    return RECORD.validate_python(
        {"cache": dict(router.cache_entries), "lookups": list(router.lookups)}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L438-L451
    )


def install_world(call: Call) -> None:
    given_cost = call["router"].get("costs")
    costs: dict[str, dict[str, JsonValue]] = {} if given_cost is None else given_cost
    litellm.model_cost = costs
    given_budget = call.get("max_budget")
    litellm.max_budget = 0.0 if given_budget is None else given_budget


async def request_answer(call: Call, router: ScriptedRouter) -> JsonValue:
    route = route_of(call)
    given_body = call.get("body")
    body: dict[str, JsonValue] = {} if given_body is None else given_body
    given_method = call.get("method")
    method = "GET" if given_method is None else given_method
    given_enforce = call.get("enforce")
    given_reject = call.get("reject")
    settings: dict[str, JsonValue] = {
        "enforce_user_param": False if given_enforce is None else given_enforce,
        "reject_clientside_metadata_tags": False if given_reject is None else given_reject,
    }
    skip = auth_checks.request_skips_budget_checks(route, call.get("model"), router)
    tags: JsonValue = None
    user: JsonValue = None
    budget: JsonValue = None
    try:
        auth_checks._reject_clientside_metadata_tags_check(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L750-L765
            settings, body, route
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L760
        tags = envelope_of(error)
    try:
        auth_checks._enforce_user_param_check(settings, http_request(method, route), body, route)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L735-L747
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L747
        user = envelope_of(error)
    try:
        auth_checks._global_proxy_budget_check(call.get("global_spend"), skip, route)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L768-L782
    except BudgetExceededError as denied:
        budget = {"denial": denial_record(denied), "envelope": envelope_of(denied)}
    return {
        "tags": tags,
        "user": user,
        "budget": budget,
        "skip": skip,
        **observed(router),
    }


async def project_answer(call: Call, router: ScriptedRouter) -> JsonValue:
    route = route_of(call)
    model = call.get("model")
    skip = auth_checks.request_skips_budget_checks(route, model, router)
    spends = call.get("spends")
    reader = CounterReads([] if spends is None else spends)
    alerts = ProjectAlerts()
    proxy = ProxyServer(PROXY_MODULE)
    proxy.get_current_spend = reader
    sys.modules[PROXY_MODULE] = proxy
    outcome = "ok"
    denial: JsonValue = None
    message: JsonValue = None
    envelope: JsonValue = None
    project = call.get("project")
    token = call.get("token")
    project_object = (
        None
        if project is None
        else TypeAdapter(LiteLLM_ProjectTableCachedObj).validate_python(project)
    )
    valid_token = None if token is None else TypeAdapter(UserAPIKeyAuth).validate_python(token)
    try:
        await auth_checks._run_project_checks(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L689-L732
            project_object, model, router, skip, valid_token, alerts
        )
    except BudgetExceededError as denied:
        outcome = "denied"
        denial = denial_record(denied)
        envelope = envelope_of(denied)
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L705-L708
        outcome = "blocked"
        message = str(error)
        envelope = envelope_of(error)
    await asyncio.sleep(0)
    return {
        "class": outcome,
        "denial": denial,
        "message": message,
        "envelope": envelope,
        "reads": list(reader.reads),
        "alerts": list(alerts.alerts),
        **observed(router),
    }


async def answer(call: Call) -> JsonValue:
    install_world(call)
    router = ScriptedRouter(call["router"])
    function = call["function"]
    model = call.get("model")
    if function == "zero_cost":
        zero = auth_checks._is_model_cost_zero(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L454
            model=model, llm_router=router
        )
        return {"zero": zero, **observed(router)}
    if function == "unpriced":
        unpriced = auth_checks._unpriced_models_in_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L672-L676
            model=model, llm_router=router
        )
        message = auth_checks._unpriced_models_block_message(unpriced)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L679-L686
        models = JSON.validate_python(list(unpriced))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L672-L676
        return {"unpriced": models, "message": message, **observed(router)}
    if function == "request_skip":
        route = route_of(call)
        return {
            "route_skips": auth_checks.route_skips_budget_checks(route),
            "request_skips": auth_checks.request_skips_budget_checks(route, model, router),
            **observed(router),
        }
    if function == "request_checks":
        return await request_answer(call, router)
    return await project_answer(call, router)


async def answer_calls(calls: list[Call]) -> JsonValue:
    answers: list[JsonValue] = []
    for call in calls:
        try:
            answers.append({"ok": True, "value": await answer(call)})
        except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L6426-L6467
            answers.append({"ok": False})
    return answers


def run(input_value: Input) -> JsonValue:
    # LiteLLM resolves sys.stdout at emit time, so records below WARNING would precede the answer
    with contextlib.redirect_stdout(io.StringIO()):
        return asyncio.run(answer_calls(input_value["calls"]))
