from __future__ import annotations

import os
from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.integrations.datadog import datadog, datadog_handler
from litellm.litellm_core_utils import dd_tracing
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.dd_span_tagger import DDSpanTagger
from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def fake_span(value: Json) -> SimpleNamespace | None:
    if not isinstance(value, dict):
        return None
    span = SimpleNamespace(trace_id=value.get("trace_id"), span_id=value.get("span_id"), tags=[])
    # LiteLLM probes these with hasattr, so a disabled method must not exist at all
    for method in ("set_tag_str", "set_tag"):
        if value.get(method) is True:
            setattr(span, method, lambda key, tag: span.tags.append([key, tag]))
    return span


def text(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def environment(env: dict[str, Json], request_tags: list[Json], metadata: dict[str, Json]) -> Json:
    # every helper call is a fresh process, so the case env replaces the inherited one
    os.environ.clear()
    os.environ.update({name: value for name, value in env.items() if isinstance(value, str)})
    payload = cast("StandardLoggingPayload", {"request_tags": request_tags, "metadata": metadata})
    return {
        "source": datadog_handler.get_datadog_source(),
        "base_url": datadog_handler.get_datadog_base_url_from_env(),
        "flags": [dd_tracing._should_use_dd_tracer(), dd_tracing._should_use_dd_profiler()],  # noqa: SLF001 - the spec
        "tags": datadog_handler.get_datadog_tags(payload),
    }


def spans(input_value: dict[str, Json]) -> Json:
    current = fake_span(input_value.get("current"))
    root = fake_span(input_value.get("root"))
    tracer = SimpleNamespace(current_span=lambda: current, current_root_span=lambda: root)
    dd_tracing.tracer = datadog.tracer = tracer
    DDSpanTagger.tag_call_id(text(input_value.get("call_id")))
    key = UserAPIKeyAuth(key_alias=text(input_value.get("key_alias")), token=text(input_value.get("key_hash")))
    DDSpanTagger.tag_request(key, text(input_value.get("requested_model")))
    payload: dict[str, Json] = {}
    logger = datadog.DataDogLogger.__new__(datadog.DataDogLogger)
    logger._add_trace_context_to_payload(cast("datadog.DatadogPayload", payload))  # noqa: SLF001 - the spec
    return {"payload": payload or None, "current_tags": current and current.tags, "root_tags": root and root.tags}


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {"function": "normalize", "values": list() as values}:
            return [datadog_handler.normalize_datadog_tag_value(value) for value in values]
        case {"function": "environment", "env": dict() as env, "request_tags": list() as tags, "metadata": dict() as meta}:
            return environment(env, tags, meta)
        case {"function": "spans"}:
            return spans(input_value)
        case _:
            raise ValueError(f"unknown datadog_tags input: {sorted(input_value)}")
