"""Oracle adapter for the shared management list planner."""

# pyright: reportAny=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportArgumentType=false, reportTypedDictNotRequiredAccess=false, reportAttributeAccessIssue=false

from collections.abc import Mapping
from dataclasses import asdict
from datetime import UTC, datetime
from typing import Any, TypedDict
from urllib.parse import parse_qsl

from litellm.proxy.list_api.common import request_validation_problem
from litellm.proxy.list_api.list_framework import (
    Compare,
    FilterSpec,
    ListSpec,
    ScopeAll,
    ScopeDenied,
    ScopeWhere,
    SortKey,
    build_query_plan,
)


class Input(TypedDict, total=False):
    function: str
    raw: str
    spec: dict[str, Any]
    params: dict[str, str]
    scope: dict[str, Any]
    errors: list[dict[str, Any]]


def _scope(value: Mapping[str, Any]) -> Any:  # noqa: ANN401
    match value.get("kind"):
        case "all":
            return ScopeAll()
        case "denied":
            return ScopeDenied(reason=str(value["reason"]))
        case "where":
            predicates = tuple(
                Compare(field=str(item["field"]), op=str(item["op"]), value=str(item["value"]))
                for item in value["predicates"]
            )
            return ScopeWhere(where=predicates)
        case other:
            raise ValueError(f"unknown scope {other!r}")


def _spec(value: Mapping[str, Any]) -> ListSpec[Any, Any]:
    filter_types: dict[str, type[Any]] = {
        "text": str,
        "int": int,
        "money": float,
        "timestamp": datetime,
    }
    filters = {
        field: FilterSpec(
            type=filter_types[str(item["value_type"])],
            ops=frozenset(str(op) for op in item["ops"]),
        )
        for field, item in dict(value["filters"]).items()
    }
    return ListSpec(
        resource=str(value["resource"]),
        sortable=frozenset(str(field) for field in value["sortable"]),
        searchable=frozenset(str(field) for field in value["searchable"]),
        filters=filters,
        default_sort=tuple(
            SortKey(field=str(item["field"]), descending=bool(item["descending"]))
            for item in value["default_sort"]
        ),
        default_page_size=int(value["default_page_size"]),
        max_page_size=int(value["max_page_size"]),
        scope=lambda _caller: _scope(value.get("scope", {"kind": "all"})),
        serialize=lambda row: row,
        tiebreaker=str(value["tiebreaker"]),
    )


def _json_value(value: Any) -> Any:  # noqa: ANN401
    if isinstance(value, datetime):
        normalized = value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)
        return normalized.isoformat().replace("+00:00", "Z")
    return value


def _predicate(value: Any) -> dict[str, Any]:  # noqa: ANN401
    result = asdict(value)
    if "value" in result:
        result["value"] = _json_value(result["value"])
    if "clauses" in result:
        result["clauses"] = [_predicate(clause) for clause in value.clauses]
    result["kind"] = type(value).__name__.lower()
    return result


def _plan(value: Any) -> dict[str, Any]:  # noqa: ANN401
    return {
        "where": [_predicate(predicate) for predicate in value.where],
        "order": [asdict(item) for item in value.order],
        "skip": value.skip,
        "take": value.take,
    }


def run(input_value: Input) -> object:
    function = input_value["function"]
    if function == "decode":
        pairs = parse_qsl(input_value["raw"], keep_blank_values=True)
        last_value = dict(pairs)
        return {"pairs": pairs, "last_value": last_value}
    if function == "plan":
        spec_value = dict(input_value["spec"])
        spec_value["scope"] = input_value.get("scope", {"kind": "all"})
        plan = build_query_plan(_spec(spec_value), input_value["params"], object())
        if hasattr(plan, "model_dump"):
            return plan.model_dump(mode="json", exclude_none=True)
        return _plan(plan)
    if function == "problem":
        errors = [
            {"type": error["error_type"], "loc": error["loc"], "msg": error["msg"]}
            for error in input_value["errors"]
        ]
        return request_validation_problem(errors).model_dump(
            mode="json", exclude_none=True
        )
    raise ValueError(f"unknown management plan function {function!r}")
