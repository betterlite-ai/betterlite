from typing import TypeAlias, cast

from litellm.proxy.management_endpoints.credential_migration import (
    LocationReport,
    _classify_callback_value,  # pyright: ignore[reportPrivateUsage]
    _classify_secret_map,  # pyright: ignore[reportPrivateUsage]
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
        category = (
            _classify_secret_map(value, key)
            if isinstance(value, dict)
            else classify_value(value, key)
        )
        report.count(category)
    for value in callbacks:
        report.count(_classify_callback_value(value))
    return cast(Json, {"counts": report.as_dict()})
