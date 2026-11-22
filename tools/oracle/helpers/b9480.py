import asyncio
import operator
import os
from functools import partial
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
from pydantic import JsonValue, TypeAdapter

TIMEOUT_VARIABLE: Final = "PROMETHEUS_BUDGET_METRICS_PER_REQUEST_TIMEOUT"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Snapshot = KeyBudgetSnapshot | TeamBudgetSnapshot | UserBudgetSnapshot | OrgBudgetSnapshot

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


def _snapshot(carried: Snapshot | None) -> JsonValue:
    if carried is None:
        return None
    return JSON.validate_json(carried.model_dump_json(by_alias=True))


def _stand_in(noop_groups: frozenset[str], group: str) -> prometheus_client.Gauge | NoOpMetric:
    if group in noop_groups:
        return NoOpMetric()
    return prometheus_client.Gauge(f"b9480_{group}_probe", "budget gauge stand-in")


class _Recorder:
    def __init__(
        self, recorded: dict[str, JsonValue], entity: str, *, has_snapshot: bool
    ) -> None:
        self._recorded = recorded
        self._entity = entity
        self._has_snapshot = has_snapshot

    async def __call__(self, *, carried: Snapshot | None = None, **fields: JsonValue) -> None:
        if self._has_snapshot:
            fields["carried"] = _snapshot(carried)
        self._recorded[self._entity] = fields


class _StandIn:
    def __init__(self, noop_groups: frozenset[str], recorded: dict[str, JsonValue]) -> None:
        self.litellm_remaining_team_budget_metric = _stand_in(noop_groups, "team")
        self.litellm_remaining_api_key_budget_metric = _stand_in(noop_groups, "key")
        self.litellm_remaining_user_budget_metric = _stand_in(noop_groups, "user")
        self.litellm_remaining_org_budget_metric = _stand_in(noop_groups, "org")
        self._customer_budget_gauges_are_noop = partial(
            operator.contains, noop_groups, "customer"
        )
        self._set_api_key_budget_metrics_after_api_request = _Recorder(
            recorded, "key", has_snapshot=True
        )
        self._set_team_budget_metrics_after_api_request = _Recorder(
            recorded, "team", has_snapshot=True
        )
        self._set_user_budget_metrics_after_api_request = _Recorder(
            recorded, "user", has_snapshot=True
        )
        self._set_org_budget_metrics_after_api_request = _Recorder(
            recorded, "org", has_snapshot=True
        )
        self._set_customer_budget_metrics_after_api_request = _Recorder(
            recorded, "customer", has_snapshot=False
        )


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
    recorded: dict[str, JsonValue] = {}
    logger = _StandIn(frozenset(noop_groups), recorded)
    asyncio.run(
        PrometheusLogger._increment_remaining_budget_metrics(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1891
            logger,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1891
            user_api_team=input_value.get("team"),
            user_api_team_alias=input_value.get("team_alias"),
            user_api_key=input_value.get("key"),
            user_api_key_alias=input_value.get("key_alias"),
            litellm_params={"metadata": metadata},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1891
            response_cost=response_cost,
            user_id=input_value.get("user_id"),
            user_api_key_org_id=input_value.get("org_id"),
            end_user_id=input_value.get("end_user_id"),
        )
    )
    return recorded


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
