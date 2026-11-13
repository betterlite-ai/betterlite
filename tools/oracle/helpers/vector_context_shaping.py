from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_logger
from litellm.integrations.vector_store_integrations.vector_store_pre_call_hook import (
    VectorStorePreCallHook,
    _configured_failure_mode,  # pyright: ignore[reportPrivateUsage]
    _requested_vector_store_ids,  # pyright: ignore[reportPrivateUsage]
    _scan_request,  # pyright: ignore[reportPrivateUsage]
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues
    from litellm.types.vector_stores import VectorStoreSearchResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_HOOK = VectorStorePreCallHook()


def _query_uses_last_message(value: dict[str, Json]) -> Json:
    messages = cast("list[AllMessageValues]", value["messages"])
    return _HOOK._extract_query_from_messages(messages)  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001


def _context_text_and_position(value: dict[str, Json]) -> Json:
    search_results = cast("list[VectorStoreSearchResponse]", value["search_results"])
    messages = cast("list[AllMessageValues]", value["messages"])
    context = _HOOK._context_messages(search_results)  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    with_context = _HOOK._messages_with_context(  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
        messages, context
    )
    return cast(
        "dict[str, Json]",
        {"context_messages": list(context), "messages_with_context": with_context},
    )


def _scan_request_merge(value: dict[str, Json]) -> Json:
    model = cast(str, value["model"])
    non_default_params = cast("dict[str, object]", value["non_default_params"])
    return cast("dict[str, Json]", dict(_scan_request(model, non_default_params)))


def _requested_ids_for_diagnostics(value: dict[str, Json]) -> Json:
    non_default_params = cast("dict[str, object]", value["non_default_params"])
    return list(_requested_vector_store_ids(non_default_params))


def _failure_policy(value: dict[str, Json]) -> Json:
    configured = value["configured"]
    logs: list[str] = []

    def capture(message: str, *args: object, **_kwargs: object) -> None:
        logs.append(message % args if args else message)

    with (
        patch.object(litellm, "vector_store_search_failure_mode", configured),
        patch.object(verbose_logger, "warning", capture),
    ):
        mode = _configured_failure_mode()
    return {"mode": mode, "warning": logs[0] if logs else None}


_HANDLERS = {
    "query_uses_last_message": _query_uses_last_message,
    "context_text_and_position": _context_text_and_position,
    "scan_request_merge": _scan_request_merge,
    "requested_ids_for_diagnostics": _requested_ids_for_diagnostics,
    "failure_policy": _failure_policy,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in _HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return _HANDLERS[operation](input_value)
