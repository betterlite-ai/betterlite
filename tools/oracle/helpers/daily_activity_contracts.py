from decimal import Decimal
from typing import TypeAlias, cast

from litellm.types.repositories.daily_activity import (
    AggregatedRows,
    DailyActivityScope,
    DailyRowsPage,
    EntityRollupRow,
    ExportRow,
    GroupingSetsRow,
    KeyMetadataRow,
    KeyPage,
    KeySpendRow,
    RollupMetricsRow,
)
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CONTRACTS = {
    "scope": DailyActivityScope, "key_spend": KeySpendRow, "key_page": KeyPage,
    "key_metadata": KeyMetadataRow, "export": ExportRow, "rollup": RollupMetricsRow,
    "grouping": GroupingSetsRow, "entity": EntityRollupRow,
    "aggregated": AggregatedRows, "daily_page": DailyRowsPage,
}
MONEY_FIELDS = frozenset({
    "spend", "flat_cost", "ptu_flat_cost", "compression_savings_spend",
    "prompt_caching_savings_spend", "gateway_injected_caching_savings_spend",
    "autorouter_savings_spend",
})


def json_value(value: object) -> Json:
    if isinstance(value, Decimal):
        result: Json = str(value)
    elif isinstance(value, dict):
        entries = cast(dict[object, object], value)
        result = {
            str(key): money_value(key, item)
            for key, item in entries.items()
        }
    elif isinstance(value, list):
        result = [json_value(item) for item in cast(list[object], value)]
    else:
        result = cast(Json, value)
    return result


def money_value(key: object, value: object) -> Json:
    if key in MONEY_FIELDS and isinstance(value, float):
        return str(value)
    return json_value(value)


def run(input_value: dict[str, Json]) -> Json:
    step = input_value.get("step")
    if not isinstance(step, dict):
        raise ValueError("step must be an object")
    kind = step.get("kind")
    value = step.get("value")
    if not isinstance(kind, str) or kind not in CONTRACTS:
        raise ValueError(f"unknown contract {kind!r}")
    adapter = TypeAdapter(CONTRACTS[kind])
    validated = adapter.validate_python(value)
    dumped = cast(object, adapter.dump_python(validated, mode="python"))
    return json_value(dumped)
