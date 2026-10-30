from collections.abc import Iterable
from typing import TypeAlias, cast

from litellm.proxy.management_endpoints.credential_migration import (
    LocationReport,
    MigrationReport,
    _classify_callback_value,  # pyright: ignore[reportPrivateUsage]
    _classify_secret_map,  # pyright: ignore[reportPrivateUsage]
    _iter_callback_var_dicts,  # pyright: ignore[reportPrivateUsage]
    classify_value,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    values = input_value.get("values")
    if not isinstance(values, list):
        raise ValueError("values must be a list")
    key = input_value.get("key", "scan")
    if not isinstance(key, str):
        raise ValueError("key must be a string")
    callbacks = input_value.get("callbacks", [])
    if not isinstance(callbacks, list):
        raise ValueError("callbacks must be a list")
    report = LocationReport(location="values")
    for value in values:
        report.count(classify_value(value, key))
    maps = input_value.get("maps", [])
    if not isinstance(maps, list):
        raise ValueError("maps must be a list")
    for value in maps:
        report.count(_classify_secret_map(value, key))
    for value in callbacks:
        report.count(_classify_callback_value(value))
    metadata = input_value.get("metadata")
    count_metadata(report, metadata)
    aggregate = MigrationReport(locations=[report])
    return cast(Json, {"counts": report.as_dict(), "report": aggregate.as_dict()})


def count_metadata(report: LocationReport, metadata: Json) -> None:
    if isinstance(metadata, dict):
        dictionaries = cast(
            Iterable[dict[str, object]],
            _iter_callback_var_dicts(cast(dict[str, object], metadata)),
        )
        for variables in dictionaries:
            for value in variables.values():
                report.count(_classify_callback_value(value))
