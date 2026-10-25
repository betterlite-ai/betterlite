from dataclasses import make_dataclass
from types import GenericAlias
from typing import TypeAlias, cast, get_type_hints

from litellm.repositories.daily_activity_sql import ExportCursor, SqlQuery
from litellm.types.repositories import daily_activity as source
from pydantic import TypeAdapter, ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
# Protocols describe attributes; derive the validation record from those source annotations.
DAILY_ROW = make_dataclass("DailyRecord", get_type_hints(source.DailyActivityRow).items())
DAILY_PAGE = make_dataclass("DailyPage", [
    (name, GenericAlias(tuple, (DAILY_ROW, Ellipsis)) if name == "rows" else kind)
    for name, kind in cast(dict[str, object], get_type_hints(source.DailyRowsPage)).items()
])
CONTRACTS = {
    "scope": source.DailyActivityScope, "key_spend": source.KeySpendRow,
    "key_page": source.KeyPage, "key_metadata": source.KeyMetadataRow,
    "export": source.ExportRow, "rollup": source.RollupMetricsRow,
    "grouping": source.GroupingSetsRow, "entity": source.EntityRollupRow,
    "aggregated": source.AggregatedRows, "daily_row": DAILY_ROW, "daily_page": DAILY_PAGE,
    "export_cursor": ExportCursor, "sql_query": SqlQuery,
}
MONEY_FIELDS = frozenset({
    "spend", "flat_cost", "ptu_flat_cost", "compression_savings_spend",
    "prompt_caching_savings_spend", "gateway_injected_caching_savings_spend",
    "autorouter_savings_spend",
})


def json_value(value: object, key: object = None) -> Json:
    if key in MONEY_FIELDS and isinstance(value, float):
        return str(value)
    if isinstance(value, dict):
        entries = cast(dict[object, object], value)
        return {str(name): json_value(item, name) for name, item in entries.items()}
    if isinstance(value, (list, tuple)):
        return [json_value(item) for item in cast(list[object] | tuple[object, ...], value)]
    return cast(Json, value)


def probe(step: Json) -> Json:
    if not isinstance(step, dict):
        raise ValueError("step must be an object")
    kind, value = step.get("kind"), step.get("value")
    if not isinstance(kind, str) or kind not in CONTRACTS or not isinstance(value, dict):
        raise ValueError(f"unknown contract {kind!r}")
    adapter = TypeAdapter[object](CONTRACTS[kind])
    output = None
    if accepts(adapter, value):
        validated = adapter.validate_python(value)
        value = cast(dict[str, Json], adapter.dump_python(validated, mode="json"))
        output = json_value(value)
    fields: dict[str, Json] = {}
    for field in value:
        changed = value.copy()
        del changed[field]
        missing = accepts(adapter, changed)
        changed[field] = None
        fields[field] = list[Json]([missing, accepts(adapter, changed)])
    return {"value": output, "fields": fields}


def accepts(adapter: TypeAdapter[object], value: object) -> bool:
    try:
        adapter.validate_python(value)
    except (ValidationError, ValueError, TypeError):
        return False
    return True


def run(input_value: dict[str, Json]) -> Json:
    steps = input_value.get("steps")
    if not isinstance(steps, dict):
        raise ValueError("steps must be an object")
    return {name: probe(step) for name, step in steps.items()}
