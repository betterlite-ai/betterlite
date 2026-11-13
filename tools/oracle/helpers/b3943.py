import json
from collections.abc import Callable
from dataclasses import asdict
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.integrations.newrelic.newrelic_metrics import (
    _metric_record_from_payload,  # pyright: ignore[reportPrivateUsage]
    build_metric_payload,
    resolve_newrelic_metric_endpoint,
)

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class InputError(ValueError):
    pass


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return cast("dict[str, Json]", value)


def _items(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return cast("list[Json]", value)


def _integer(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def _to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def _payload(value: Json) -> "StandardLoggingPayload":
    return cast("StandardLoggingPayload", _object(value, "payload"))


def _region(input_value: dict[str, Json]) -> Json:
    region = input_value.get("region")
    if region is not None and not isinstance(region, str):
        raise InputError("region must be a string or null")
    return _to_json({"endpoint": resolve_newrelic_metric_endpoint(region)})


def _record(input_value: dict[str, Json]) -> Json:
    record = _metric_record_from_payload(_payload(input_value.get("payload")))
    return _to_json(asdict(record))


def _envelope(input_value: dict[str, Json]) -> Json:
    payloads = _items(input_value.get("records"), "records")
    records = tuple(_metric_record_from_payload(_payload(value)) for value in payloads)
    window_start_ms = _integer(input_value.get("window_start_ms"), "window_start_ms")
    now_ms = _integer(input_value.get("now_ms"), "now_ms")
    envelopes = build_metric_payload(
        records, window_start=window_start_ms / 1000.0, now=now_ms / 1000.0
    )
    return _to_json(envelopes)


_HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "region": _region,
    "record": _record,
    "payload": _envelope,
}


def _operation(input_value: dict[str, Json]) -> str:
    operation = input_value.get("operation")
    if not isinstance(operation, str):
        raise InputError("operation must be a string")
    return operation


def run(input_value: dict[str, Json]) -> Json:
    operation = _operation(input_value)
    handler = _HANDLERS.get(operation)
    if handler is None:
        raise InputError(f"unknown newrelic_metrics operation {operation}")
    return handler(input_value)
