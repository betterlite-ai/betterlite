import asyncio
from datetime import UTC, datetime
from typing import TypeAlias, cast

from litellm.proxy.list_api.in_memory import InMemoryListExecutor
from litellm.proxy.list_api.list_framework import (
    AnyOf,
    Compare,
    ComparisonOp,
    IsNull,
    QueryPlan,
    SortKey,
    Within,
    order_by_sql,
    where_sql,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Cell: TypeAlias = str | int | float | datetime | None
CellValue: TypeAlias = Cell | tuple[Cell, ...]


def object_value(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


def sequence_value(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected list")
    return value


def sort_keys(raw: dict[str, Json]) -> tuple[SortKey, ...]:
    result: list[SortKey] = []
    for value in sequence_value(raw.get("order", [])):
        item = object_value(value)
        result.append(SortKey(str(item["field"]), item.get("descending") is True))
    return tuple(result)


def predicate(raw_value: Json) -> Compare | Within | IsNull | AnyOf:
    raw = object_value(raw_value)
    kind = raw["kind"]
    field = str(raw.get("field", ""))
    if kind == "compare":
        value = raw["value"]
        if isinstance(value, str) and raw.get("value_type") == "timestamp":
            value = datetime.fromisoformat(value)
        comparable = cast(str | int | float | datetime, value)
        return Compare(field, cast(ComparisonOp, raw["op"]), comparable)
    if kind == "within":
        values = tuple(cast(str | int | float, item) for item in sequence_value(raw["values"]))
        return Within(field, values)
    if kind == "is_null":
        return IsNull(field, raw.get("negated") is True)
    if kind == "any_of":
        clauses = tuple(cast(Compare, predicate(item)) for item in sequence_value(raw["clauses"]))
        return AnyOf(clauses)
    raise ValueError(f"unknown predicate kind: {kind}")


def cell_values(row: dict[str, Json]) -> dict[str, Cell | tuple[Cell, ...]]:
    return {name: cell_value(value) for name, value in row.items()}


def cell_value(value: Json) -> CellValue:
    if isinstance(value, list):
        return tuple(cast(Cell, item) for item in value)
    return cast(Cell, value)


def json_value(value: object) -> Json:
    if isinstance(value, datetime):
        return value.astimezone(UTC).isoformat()
    return cast(Json, value)


def evaluate(raw_value: Json) -> Json:
    raw = object_value(raw_value)
    where = tuple(predicate(item) for item in sequence_value(raw.get("where", [])))
    function = raw.get("function")
    order = sort_keys(raw)
    if function == "sql":
        sql, params = where_sql(where, int(cast(int | str, raw.get("first_index", 1))))
        return {
            "where": sql,
            "params": [json_value(value) for value in params],
            "order": order_by_sql(order),
        }
    if function == "memory":
        rows = tuple(object_value(value) for value in sequence_value(raw.get("rows", [])))
        executor: InMemoryListExecutor[dict[str, Json]] = InMemoryListExecutor(
            rows=rows,
            cells=cell_values,
        )

        async def execute() -> Json:
            matched = await executor.count(where)
            plan = QueryPlan(where=where, order=order, skip=0, take=len(rows))
            selected = await executor.find_many(plan)
            field = raw.get("distinct_field")
            distinct = await executor.distinct(str(field), where) if isinstance(field, str) else ()
            return {
                "count": matched,
                "rows": cast(Json, list(selected)),
                "distinct": cast(Json, list(distinct)),
            }

        return asyncio.run(execute())
    raise ValueError(f"unknown function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    cases = object_value(input_value.get("cases"))
    return {name: evaluate(case) for name, case in cases.items()}
