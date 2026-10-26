from __future__ import annotations

import os
from collections.abc import Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.integrations.datadog import datadog, datadog_handler
from litellm.litellm_core_utils import dd_tracing
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.dd_span_tagger import DDSpanTagger

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


TagCallback: TypeAlias = Callable[[str, str], None]


@dataclass
class SpanStorage:
    trace_id: Json
    span_id: Json
    tags: list[list[str]]


@dataclass
class TracerStorage:
    current_span: Callable[[], SpanStorage | None]
    current_root_span: Callable[[], SpanStorage | None]


def record_tag(storage: SpanStorage, key: str, tag: str) -> None:
    storage.tags.append([key, tag])


def fake_span(value: Json) -> SpanStorage | None:
    if not isinstance(value, dict):
        return None
    span = SpanStorage(
        trace_id=value.get("trace_id"), span_id=value.get("span_id"), tags=[]
    )
    # LiteLLM probes these with hasattr, so a disabled method must not exist at all
    for method in ("set_tag_str", "set_tag"):
        if value.get(method) is True:
            def callback(key: str, tag: str) -> None:
                record_tag(span, key, tag)

            setattr(span, method, callback)
    return span


def text(value: Json) -> str | None:
    return value if isinstance(value, str) else None


def environment(
    env: dict[str, Json], request_tags: list[Json], metadata: dict[str, Json]
) -> Json:
    # every helper call is a fresh process, so the case env replaces the inherited one
    os.environ.clear()
    os.environ.update(
        {name: value for name, value in env.items() if isinstance(value, str)}
    )
    payload = cast(
        "StandardLoggingPayload",
        cast(object, {"request_tags": request_tags, "metadata": metadata}),
    )
    return cast(Json, {
        "source": datadog_handler.get_datadog_source(),
        "base_url": datadog_handler.get_datadog_base_url_from_env(),
        "flags": [
            dd_tracing._should_use_dd_tracer(),  # noqa: SLF001 - the spec
            dd_tracing._should_use_dd_profiler(),  # noqa: SLF001 - the spec
        ],
        "tags": datadog_handler.get_datadog_tags(payload),
    })


def spans(input_value: dict[str, Json]) -> Json:
    current = fake_span(input_value.get("current"))
    root = fake_span(input_value.get("root"))
    def current_span() -> SpanStorage | None:
        return current

    def current_root_span() -> SpanStorage | None:
        return root

    tracer = TracerStorage(current_span=current_span, current_root_span=current_root_span)
    dd_tracing.tracer = datadog.tracer = tracer
    DDSpanTagger.tag_call_id(text(input_value.get("call_id")))
    key = UserAPIKeyAuth(
        key_alias=text(input_value.get("key_alias")),
        token=text(input_value.get("key_hash")),
    )
    DDSpanTagger.tag_request(key, text(input_value.get("requested_model")))
    payload: dict[str, Json] = {}
    logger = datadog.DataDogLogger.__new__(datadog.DataDogLogger)
    logger._add_trace_context_to_payload(  # noqa: SLF001 - the spec
        cast("datadog.DatadogPayload", cast(object, payload))
    )
    return cast(Json, {
        "payload": payload or None,
        "current_tags": current and current.tags,
        "root_tags": root and root.tags,
    })


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {"function": "normalize", "values": list() as values}:
            return [datadog_handler.normalize_datadog_tag_value(value) for value in values]
        case {
            "function": "environment",
            "env": dict() as env,
            "request_tags": list() as tags,
            "metadata": dict() as meta,
        }:
            return environment(env, tags, meta)
        case {"function": "spans"}:
            return spans(input_value)
        case _:
            raise ValueError(f"unknown datadog_tags input: {sorted(input_value)}")
