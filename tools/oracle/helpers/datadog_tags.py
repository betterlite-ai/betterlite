from __future__ import annotations

import os
from contextlib import contextmanager
from typing import Generator, TypeAlias, cast

from litellm.integrations.datadog.datadog_handler import (
    get_datadog_tags,
    normalize_datadog_tag_value,
)
from litellm.litellm_core_utils import dd_tracing

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
SPAN_ENV = ("DD_SOURCE", "DD_SERVICE", "HOSTNAME", "DD_ENV", "POD_NAME", "DD_VERSION", "DD_BASE_URL")


class FakeSpan:
    def __init__(self, value: dict[str, Json]) -> None:
        self.available = value.get("available") is True
        self.trace_id = value.get("trace_id")
        self.span_id = value.get("span_id")
        self.has_set_tag_str = value.get("set_tag_str") is True
        self.has_set_tag = value.get("set_tag") is True
        self.tags: list[tuple[str, str]] = []

    def __getattribute__(self, name: str) -> object:
        if name == "set_tag_str" and not object.__getattribute__(self, "has_set_tag_str"):
            raise AttributeError(name)
        if name == "set_tag" and not object.__getattribute__(self, "has_set_tag"):
            raise AttributeError(name)
        return object.__getattribute__(self, name)

    def set_tag_str(self, key: str, value: str) -> None:
        self.tags.append((key, value))

    def set_tag(self, key: str, value: str) -> None:
        self.tags.append((key, value))


class FakeTracer:
    def __init__(self, current: FakeSpan | None, root: FakeSpan | None) -> None:
        self.current = current
        self.root = root

    def current_span(self) -> FakeSpan | None:
        return self.current if self.current is not None and self.current.available else None

    def current_root_span(self) -> FakeSpan | None:
        return self.root if self.root is not None and self.root.available else None


def span(value: Json) -> FakeSpan | None:
    if value is None or not isinstance(value, dict):
        return None
    return FakeSpan(value)


@contextmanager
def configured_environment(values: dict[str, Json]) -> Generator[None]:
    saved = {name: os.environ.get(name) for name in SPAN_ENV}
    for name in SPAN_ENV:
        os.environ.pop(name, None)
        value = values.get(name)
        if isinstance(value, str):
            os.environ[name] = value
    try:
        yield
    finally:
        for name, value in saved.items():
            os.environ.pop(name, None)
            if value is not None:
                os.environ[name] = value


def context(current: FakeSpan | None, root: FakeSpan | None) -> dict[str, str] | None:
    tracer = FakeTracer(current, root)
    saved = dd_tracing.tracer
    dd_tracing.tracer = cast("object", tracer)
    try:
        active = dd_tracing.get_active_span()
        if active is None or active.trace_id is None:
            return None
        result = {"trace_id": str(active.trace_id)}
        if active.span_id is not None:
            result["span_id"] = str(active.span_id)
        return result
    finally:
        dd_tracing.tracer = saved


def tagging(
    current: FakeSpan | None,
    root: FakeSpan | None,
    values: dict[str, Json],
) -> dict[str, Json]:
    tracer = FakeTracer(current, root)
    saved = dd_tracing.tracer
    dd_tracing.tracer = cast("object", tracer)
    try:
        tagged: list[str] = []
        for key, field in (
            ("litellm.call_id", "call_id"),
            ("litellm.key_alias", "key_alias"),
            ("litellm.key_hash", "key_hash"),
            ("litellm.requested_model", "requested_model"),
        ):
            value = values.get(field)
            if isinstance(value, str) and dd_tracing.set_active_span_tag(key, value):
                tagged.append(key)
        return {"tags": tagged, "context": context(current, root)}
    finally:
        dd_tracing.tracer = saved


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if not isinstance(function, str):
        raise ValueError("function must be a string")
    if function == "normalize":
        values = input_value.get("values")
        if not isinstance(values, list) or not all(isinstance(value, str) for value in values):
            raise ValueError("values must be a list of strings")
        return [normalize_datadog_tag_value(value) for value in values]
    if function == "environment":
        with configured_environment(map_value(input_value, "env")):
            return {
                "source": os.getenv("DD_SOURCE", "litellm"),
                "service": os.getenv("DD_SERVICE", "litellm-server"),
                "hostname": os.getenv("HOSTNAME", ""),
                "environment": os.getenv("DD_ENV", "unknown"),
                "pod_name": os.getenv("POD_NAME", "unknown"),
                "version": os.getenv("DD_VERSION", "unknown"),
                "base_url": os.getenv("DD_BASE_URL"),
            }
    if function == "tags":
        with configured_environment(map_value(input_value, "env")):
            logging_object = {
                "request_tags": input_value.get("request_tags", []),
                "metadata": input_value.get("metadata", {}),
            }
            return get_datadog_tags(cast("object", logging_object))
    if function in {"trace_context", "span_tagging"}:
        current = span(input_value.get("current"))
        root = span(input_value.get("root"))
        if function == "trace_context":
            return context(current, root)
        return tagging(current, root, input_value)
    raise ValueError(f"unknown datadog_tags function: {function}")


def map_value(input_value: dict[str, Json], name: str) -> dict[str, Json]:
    value = input_value.get(name)
    if not isinstance(value, dict) or not all(isinstance(key, str) for key in value):
        raise ValueError(f"{name} must be an object")
    return value
