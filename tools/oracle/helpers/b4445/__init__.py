from datetime import datetime
from typing import Final, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy.health_endpoints import _health_endpoints as health  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L615-L642
from pydantic import JsonValue, TypeAdapter


class Row(TypedDict):
    health_check_id: str
    model_name: str
    model_id: str | None
    status: str
    healthy_count: int
    unhealthy_count: int
    error_message: str | None
    response_time_ms: float | None
    details: JsonValue
    checked_by: str | None
    checked_at: str
    created_at: str


class ConvertInput(TypedDict):
    function: str
    rows: list[Row]


class Deployment(TypedDict):
    model_name: str
    model_id: str


class Router(TypedDict):
    deployments: list[Deployment]
    fail: bool


class Case(TypedDict):
    model: str | None
    model_id: str | None
    router: Router | None


class ResolveInput(TypedDict):
    function: str
    cases: list[Case]


class ErrorDetail(TypedDict):
    error: str


class Input(TypedDict):
    function: str


class CheckRecord:
    def __init__(self, row: Row) -> None:
        self.health_check_id = row["health_check_id"]
        self.model_name = row["model_name"]
        self.model_id = row["model_id"]
        self.status = row["status"]
        self.healthy_count = row["healthy_count"]
        self.unhealthy_count = row["unhealthy_count"]
        self.error_message = row["error_message"]
        self.response_time_ms = row["response_time_ms"]
        self.details = row["details"]
        self.checked_by = row["checked_by"]
        self.checked_at = datetime.fromisoformat(row["checked_at"])
        self.created_at = datetime.fromisoformat(row["created_at"])


class ModelDeployment:
    def __init__(self, model_name: str) -> None:
        self.model_name = model_name


class FakeRouter:
    def __init__(self, router: Router) -> None:
        self.deployments = router["deployments"]
        self.fail = router["fail"]

    def get_deployment(self, model_id: str) -> ModelDeployment | None:
        if self.fail:
            raise RuntimeError("router lookup failed")
        for deployment in self.deployments:
            if deployment["model_id"] == model_id:
                return ModelDeployment(deployment["model_name"])
        return None


CONVERT_INPUT: Final[TypeAdapter[ConvertInput]] = TypeAdapter(ConvertInput)
RESOLVE_INPUT: Final[TypeAdapter[ResolveInput]] = TypeAdapter(ResolveInput)
ERROR_DETAIL: Final[TypeAdapter[ErrorDetail]] = TypeAdapter(ErrorDetail)


def convert_rows(value: ConvertInput) -> JsonValue:
    converted: list[JsonValue] = []
    for row in value["rows"]:
        record = CheckRecord(row)
        item: dict[str, JsonValue] = health._convert_health_check_to_dict(record)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L615-L630
        converted.append(item)
    return converted


def target_answer(case: Case) -> JsonValue:
    router = None if case["router"] is None else FakeRouter(case["router"])
    try:
        model_name = health._health_endpoint_resolve_target_model_name(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L1090-L1120
            case["model"],
            case["model_id"],
            router,
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/health_endpoints/_health_endpoints.py#L1090-L1120
        detail = ERROR_DETAIL.validate_python(error.detail)
        return {"error": {"status": error.status_code, "detail": {"error": detail["error"]}}}
    return {"model_name": model_name}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    operation = input_value["function"]
    if operation == "convert_row":
        return convert_rows(CONVERT_INPUT.validate_python(input_value))
    if operation == "resolve_target":
        resolve_input = RESOLVE_INPUT.validate_python(input_value)
        return [target_answer(case) for case in resolve_input["cases"]]
    raise ValueError(f"unknown function: {operation}")
