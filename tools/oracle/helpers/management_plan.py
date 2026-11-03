"""Oracle adapter for the shared management list planner."""

from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict, cast
from urllib.parse import parse_qsl

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.validation_error_body import ValidationErrorDetail
from litellm.proxy.list_api.common import request_validation_problem
from litellm.proxy.list_api.list_framework import (
    Compare,
    FilterOp,
    FilterSpec,
    FilterType,
    IsNull,
    ListSpec,
    Predicate,
    QueryPlan,
    ScopeAll,
    ScopeDenied,
    ScopeWhere,
    SortKey,
    Within,
    build_query_plan,
)
from litellm.types.proxy.management_endpoints.management_v1 import ProblemDetail

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ComparisonInput = Literal["eq", "contains", "gte", "lte", "gt", "lt", "not"]
FilterTypeName = Literal["text", "int", "money", "timestamp"]


class CompareInput(TypedDict):
    field: str
    op: ComparisonInput
    value: str


class IsNullInput(TypedDict):
    field: str
    negated: bool


class ScopeAllInput(TypedDict):
    kind: Literal["all"]


class ScopeDeniedInput(TypedDict):
    kind: Literal["denied"]
    reason: str


class ScopeWhereInput(TypedDict):
    kind: Literal["where"]
    predicates: list[CompareInput | IsNullInput]


ScopeInput = ScopeAllInput | ScopeDeniedInput | ScopeWhereInput


class FilterInput(TypedDict):
    value_type: FilterTypeName
    ops: list[FilterOp]


class SortInput(TypedDict):
    field: str
    descending: bool


class SpecInput(TypedDict):
    resource: str
    sortable: list[str]
    searchable: list[str]
    filters: dict[str, FilterInput]
    default_sort: list[SortInput]
    default_page_size: int
    max_page_size: int
    tiebreaker: str


class DecodeInput(TypedDict):
    function: Literal["decode"]
    raw: str


class PlanInput(TypedDict):
    function: Literal["plan"]
    spec: SpecInput
    params: dict[str, str]
    scope: NotRequired[ScopeInput]


class ErrorInput(TypedDict):
    error_type: str
    loc: list[str | int]
    msg: str


class ProblemInput(TypedDict):
    function: Literal["problem"]
    errors: list[ErrorInput]


Input = DecodeInput | PlanInput | ProblemInput


def _scope(value: ScopeInput) -> ScopeAll | ScopeDenied | ScopeWhere:
    match value["kind"]:
        case "all":
            return ScopeAll()
        case "denied":
            return ScopeDenied(reason=value["reason"])
        case "where":
            predicates = tuple(
                Compare(field=item["field"], op=item["op"], value=item["value"])
                if "op" in item
                else IsNull(field=item["field"], negated=item["negated"])
                for item in value["predicates"]
            )
            return ScopeWhere(where=predicates)


def _filter_type(value: FilterTypeName) -> FilterType:
    match value:
        case "text":
            return str
        case "int":
            return int
        case "money":
            return float
        case "timestamp":
            return datetime


def _spec(value: SpecInput, scope_input: ScopeInput) -> ListSpec[Json, Json]:
    filters = {
        field: FilterSpec(
            type=_filter_type(item["value_type"]),
            ops=frozenset(item["ops"]),
        )
        for field, item in value["filters"].items()
    }
    return ListSpec(
        resource=value["resource"],
        sortable=frozenset(value["sortable"]),
        searchable=frozenset(value["searchable"]),
        filters=filters,
        default_sort=tuple(
            SortKey(field=item["field"], descending=item["descending"])
            for item in value["default_sort"]
        ),
        default_page_size=value["default_page_size"],
        max_page_size=value["max_page_size"],
        scope=lambda _caller: _scope(scope_input),
        serialize=lambda row: row,
        tiebreaker=value["tiebreaker"],
    )


def _json_value(value: str | float | datetime) -> Json:
    if isinstance(value, datetime):
        normalized = value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)
        return normalized.isoformat().replace("+00:00", "Z")
    return value


def _predicate(value: Predicate) -> dict[str, Json]:
    if isinstance(value, Compare):
        return {
            "kind": "compare",
            "field": value.field,
            "op": value.op,
            "value": _json_value(value.value),
        }
    if isinstance(value, Within):
        return {
            "kind": "within",
            "field": value.field,
            "values": [_json_value(item) for item in value.values],
        }
    if isinstance(value, IsNull):
        return {"kind": "isnull", "field": value.field, "negated": value.negated}
    return {"kind": "anyof", "clauses": [_predicate(item) for item in value.clauses]}


def _plan(value: QueryPlan) -> dict[str, Json]:
    return {
        "where": [_predicate(predicate) for predicate in value.where],
        "order": [
            {"field": item.field, "descending": item.descending} for item in value.order
        ],
        "skip": value.skip,
        "take": value.take,
    }


def _problem(problem: ProblemDetail) -> dict[str, Json]:
    result: dict[str, Json] = {
        "type": problem.type,
        "title": problem.title,
        "status": problem.status,
        "detail": problem.detail,
    }
    if problem.allowed is not None:
        result["allowed"] = list(problem.allowed)
    return result


def _decode(value: DecodeInput) -> dict[str, Json]:
    pairs = parse_qsl(value["raw"], keep_blank_values=True)
    return {
        "pairs": [[key, item] for key, item in pairs],
        "last_value": dict(pairs),
    }


def _plan_result(value: PlanInput) -> dict[str, Json]:
    caller_scope = value.get("scope", ScopeAllInput(kind="all"))
    spec = _spec(value["spec"], caller_scope)
    plan = build_query_plan(
        spec,
        value["params"],
        cast(UserAPIKeyAuth, object()),
    )
    return _problem(plan) if isinstance(plan, ProblemDetail) else _plan(plan)


def _validation_result(value: ProblemInput) -> dict[str, Json]:
    errors: list[ValidationErrorDetail] = [
        ValidationErrorDetail(
            type=error["error_type"],
            loc=tuple(error["loc"]),
            msg=error["msg"],
        )
        for error in value["errors"]
    ]
    return _problem(request_validation_problem(errors))


def run(input_value: Input) -> Json:
    if input_value["function"] == "decode":
        return _decode(input_value)
    if input_value["function"] == "plan":
        return _plan_result(input_value)
    return _validation_result(input_value)
