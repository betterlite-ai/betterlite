from datetime import datetime, tzinfo
from typing import ClassVar, Final, Self, TypeAlias, TypedDict

import litellm
from litellm.proxy.db.health_check_latest import LatestHealthCheckRow
from litellm.proxy.health_endpoints import _health_endpoints as health  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L679-L767
from litellm.proxy.health_endpoints._health_endpoints import _AggregatedHealthResult  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L770-L777
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Deployment(TypedDict, total=False):
    model_name: str | None
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class ModelParam(TypedDict):
    model_name: str
    model_id: str | None


class Result(TypedDict):
    model_name: str
    model_id: str | None
    healthy_count: int
    unhealthy_count: int
    error_message: str | None


class Row(TypedDict):
    health_check_id: str
    model_name: str
    model_id: str | None
    status: str
    checked_at: str
    created_at: str
    updated_at: str


class IndexInput(TypedDict):
    deployments: list[Deployment]


class AggregateInput(TypedDict):
    deployments: list[Deployment]
    healthy: list[dict[str, JsonValue]]
    unhealthy: list[dict[str, JsonValue]]


class DecisionInput(TypedDict):
    result: Result
    latest: list[Row]
    now: str


class TimeInput(TypedDict):
    values: list[float | str | None]


class Input(TypedDict):
    function: str


INDEX_INPUT: Final[TypeAdapter[IndexInput]] = TypeAdapter(IndexInput)
AGGREGATE_INPUT: Final[TypeAdapter[AggregateInput]] = TypeAdapter(AggregateInput)
DECISION_INPUT: Final[TypeAdapter[DecisionInput]] = TypeAdapter(DecisionInput)
TIME_INPUT: Final[TypeAdapter[TimeInput]] = TypeAdapter(TimeInput)
ROWS: Final[TypeAdapter[list[LatestHealthCheckRow]]] = TypeAdapter(list[LatestHealthCheckRow])
AggregatedResults: TypeAlias = dict[tuple[str | None, str], _AggregatedHealthResult]


class FrozenDatetime(datetime):
    instant: ClassVar[datetime]

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


def model_param_index(value: IndexInput) -> JsonValue:
    mapping: dict[str, list[ModelParam]] = health._build_model_param_to_info_mapping(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L679-L708
        value["deployments"],
    )
    output: dict[str, JsonValue] = {}
    for param, models in mapping.items():
        listed: list[JsonValue] = [
            {"model_name": model["model_name"], "model_id": model["model_id"]}
            for model in models
        ]
        output[param] = listed
    return output


def aggregate_results(value: AggregateInput) -> JsonValue:
    deployments = value["deployments"]
    mapping: dict[str, list[ModelParam]] = health._build_model_param_to_info_mapping(deployments)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L679-L708
    results: AggregatedResults = health._aggregate_health_check_results(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L711-L767
        mapping,
        value["healthy"],
        value["unhealthy"],
    )
    entries: list[JsonValue] = [
        {
            "model_name": result["model_name"],
            "model_id": result["model_id"],
            "healthy_count": result["healthy_count"],
            "unhealthy_count": result["unhealthy_count"],
            "error_message": result["error_message"],
        }
        for result in results.values()
    ]
    return entries


def persist_decision(value: DecisionInput) -> JsonValue:
    latest: dict[str, LatestHealthCheckRow] = {}
    for row in ROWS.validate_python(value["latest"]):
        latest.setdefault(row.model_id or row.model_name, row)
    FrozenDatetime.instant = datetime.fromisoformat(value["now"])
    health.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L4
    try:
        return health._should_persist_health_check_result(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L784-L796
            value["result"],
            latest,
        )
    finally:
        health.datetime = datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L4


def response_times(value: TimeInput) -> JsonValue:
    client = PrismaClient.__new__(PrismaClient)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4395
    return [
        client._validate_response_time(item)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L6894-L6903
        for item in value["values"]
    ]


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    operation = input_value["function"]
    if operation == "model_param_index":
        return model_param_index(INDEX_INPUT.validate_python(input_value))
    if operation == "aggregate_results":
        return aggregate_results(AGGREGATE_INPUT.validate_python(input_value))
    if operation == "persist_decision":
        return persist_decision(DECISION_INPUT.validate_python(input_value))
    if operation == "response_time":
        return response_times(TIME_INPUT.validate_python(input_value))
    raise ValueError(f"unknown function: {operation}")
