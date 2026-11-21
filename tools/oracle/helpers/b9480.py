import asyncio
import os
from typing import Final, Literal, NotRequired, TypedDict, TypeVar

import prometheus_client
from litellm.integrations.prometheus import (
    NoOpMetric,
    PrometheusLogger,
    _get_budget_metrics_per_request_timeout,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L191
)
from litellm.types.proxy.carried_budget_state import (
    KeyBudgetSnapshot,
    OrgBudgetSnapshot,
    TeamBudgetSnapshot,
    UserBudgetSnapshot,
)
from pydantic import BaseModel, JsonValue, TypeAdapter

# ruff: noqa: SLF001 - the case runs LiteLLM's own private fan-out  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1891

TIMEOUT_VARIABLE: Final = "PROMETHEUS_BUDGET_METRICS_PER_REQUEST_TIMEOUT"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

T = TypeVar("T")


class Input(TypedDict):
    function: str
    values: NotRequired[list[str | None]]
    team: NotRequired[str | None]
    team_alias: NotRequired[str | None]
    key: NotRequired[str | None]
    key_alias: NotRequired[str | None]
    user_id: NotRequired[str | None]
    org_id: NotRequired[str | None]
    end_user_id: NotRequired[str | None]
    response_cost: NotRequired[float]
    metadata: NotRequired[dict[str, JsonValue]]
    noop_groups: NotRequired[list[Literal["team", "key", "user", "org", "customer"]]]


def _snapshot(carried: BaseModel | None) -> JsonValue:
    if carried is None:
        return None
    return JSON.validate_json(carried.model_dump_json(by_alias=True))


def _stand_in(noop_groups: frozenset[str], group: str) -> prometheus_client.Gauge | NoOpMetric:
    if group in noop_groups:
        return NoOpMetric()
    return prometheus_client.Gauge(f"b9480_{group}_probe", "budget gauge stand-in")


class ProbeLogger(PrometheusLogger):
    def __init__(self, noop_groups: frozenset[str]) -> None:
        self._noop_groups = noop_groups
        self._recorded: dict[str, JsonValue] = {}
        self.litellm_remaining_team_budget_metric = _stand_in(noop_groups, "team")
        self.litellm_remaining_api_key_budget_metric = _stand_in(noop_groups, "key")
        self.litellm_remaining_user_budget_metric = _stand_in(noop_groups, "user")
        self.litellm_remaining_org_budget_metric = _stand_in(noop_groups, "org")

    def _customer_budget_gauges_are_noop(self) -> bool:  # ast-grep-ignore: no-long-names
        return "customer" in self._noop_groups

    def recorded(self) -> dict[str, JsonValue]:
        return self._recorded

    async def _set_api_key_budget_metrics_after_api_request(  # ast-grep-ignore: no-long-names  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L4312
        self,
        user_api_key: str | None,
        user_api_key_alias: str | None,
        response_cost: float,
        key_max_budget: float | None,
        key_spend: float | None,
        carried: KeyBudgetSnapshot | None = None,
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "key": user_api_key,
            "key_alias": user_api_key_alias,
            "response_cost": response_cost,
            "max_budget": key_max_budget,
            "spend": key_spend,
            "carried": _snapshot(carried),
        }
        self._recorded["key"] = recorded

    async def _set_team_budget_metrics_after_api_request(  # ast-grep-ignore: no-long-names  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3936
        self,
        user_api_team: str | None,
        user_api_team_alias: str | None,
        team_spend: float | None,
        team_max_budget: float | None,
        response_cost: float,
        carried: TeamBudgetSnapshot | None = None,
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "team": user_api_team,
            "team_alias": user_api_team_alias,
            "spend": team_spend,
            "max_budget": team_max_budget,
            "response_cost": response_cost,
            "carried": _snapshot(carried),
        }
        self._recorded["team"] = recorded

    async def _set_user_budget_metrics_after_api_request(  # ast-grep-ignore: no-long-names  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L4380
        self,
        user_id: str | None,
        user_spend: float | None,
        user_max_budget: float | None,
        response_cost: float,
        carried: UserBudgetSnapshot | None = None,
        user_email: str | None = None,
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "user_id": user_id,
            "spend": user_spend,
            "max_budget": user_max_budget,
            "response_cost": response_cost,
            "carried": _snapshot(carried),
            "user_email": user_email,
        }
        self._recorded["user"] = recorded

    async def _set_org_budget_metrics_after_api_request(  # ast-grep-ignore: no-long-names
        self,
        org_id: str | None,
        response_cost: float,
        carried: OrgBudgetSnapshot | None = None,
        org_alias: str | None = None,
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "org_id": org_id,
            "response_cost": response_cost,
            "carried": _snapshot(carried),
            "org_alias": org_alias,
        }
        self._recorded["org"] = recorded

    async def _set_customer_budget_metrics_after_api_request(  # ast-grep-ignore: no-long-names
        self,
        end_user_id: str | None,
        response_cost: float,
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "end_user_id": end_user_id,
            "response_cost": response_cost,
        }
        self._recorded["customer"] = recorded


def _resolved_seconds(value: str | None) -> float:
    if value is None:
        os.environ.pop(TIMEOUT_VARIABLE, None)
    else:
        os.environ[TIMEOUT_VARIABLE] = value
    return _get_budget_metrics_per_request_timeout()


def timeout(values: list[str | None]) -> JsonValue:
    resolved = [_resolved_seconds(value) for value in values]
    return JSON.validate_python(resolved)


def fan_out(input_value: Input) -> JsonValue:
    metadata = required(input_value.get("metadata"), "fan_out", "metadata")
    noop_groups = required(input_value.get("noop_groups"), "fan_out", "noop_groups")
    response_cost = required(input_value.get("response_cost"), "fan_out", "response_cost")
    logger = ProbeLogger(frozenset(noop_groups))
    asyncio.run(
        PrometheusLogger._increment_remaining_budget_metrics(  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1891
            logger,
            user_api_team=input_value.get("team"),
            user_api_team_alias=input_value.get("team_alias"),
            user_api_key=input_value.get("key"),
            user_api_key_alias=input_value.get("key_alias"),
            litellm_params={"metadata": metadata},
            response_cost=response_cost,
            user_id=input_value.get("user_id"),
            user_api_key_org_id=input_value.get("org_id"),
            end_user_id=input_value.get("end_user_id"),
        )
    )
    return logger.recorded()


def required(value: T | None, function: str, name: str) -> T:
    if value is None:
        raise ValueError(f"the {function} function needs a {name}")
    return value


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "timeout":
        return timeout(required(input_value.get("values"), function, "values"))
    if function == "fan_out":
        return fan_out(input_value)
    raise ValueError(f"unsupported b9480 function {function}")
