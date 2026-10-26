from collections.abc import Mapping
from typing import TypeAlias
from unittest.mock import patch

from litellm.proxy import proxy_server
from litellm.proxy.hooks.parallel_request_limiter_v3 import ParallelRequestLimiter
from litellm.types.llms.openai import ResponseAPIUsage
from litellm.types.rerank import RerankResponse
from litellm.types.utils import Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def typed_usage(value: Object) -> Usage:
    return Usage(**value)


def typed_response_api_usage(value: Object) -> ResponseAPIUsage:
    return ResponseAPIUsage(**value)


def response_object(input_value: Object) -> object:
    usage_kind = input_value.get("usage_kind")
    if usage_kind == "response_api":
        value = input_value.get("response_api_usage", {})
        if not isinstance(value, dict):
            raise TypeError("response_api_usage must be an object")
        return typed_response_api_usage(value)
    if usage_kind == "mapping":
        value = input_value.get("usage", {})
        if not isinstance(value, dict):
            raise TypeError("usage must be an object")
        return {"usage": value}
    if usage_kind == "missing":
        return {}
    value = input_value.get("usage", {})
    if not isinstance(value, dict):
        raise TypeError("usage must be an object")
    return typed_usage(value)


def rerank_response(input_value: Object) -> RerankResponse:
    metadata: dict[str, object] = {}
    tokens = input_value.get("rerank_tokens")
    if isinstance(tokens, dict):
        metadata["tokens"] = tokens
    billed_units = input_value.get("billed_units_total_tokens")
    if isinstance(billed_units, int):
        metadata["billed_units"] = {"total_tokens": billed_units}
    return RerankResponse(meta=metadata or None)


def run(input_value: Object) -> dict[str, Json]:
    limiter = object.__new__(ParallelRequestLimiter)
    response = response_object(input_value)
    rerank = rerank_response(input_value)
    configured = input_value.get("rate_limit_type")
    settings = {"token_rate_limit_type": configured} if configured is not None else {}
    with patch.object(proxy_server, "general_settings", settings):
        rate_limit_type = limiter.get_rate_limit_type()
        usage = limiter._response_usage(response)  # pyright: ignore[reportPrivateUsage]
        rerank_usage = limiter._resolve_rerank_token_usage(rerank)  # pyright: ignore[reportPrivateUsage]
        if rerank_usage is None:
            total_tokens = limiter._get_total_tokens_from_usage(  # pyright: ignore[reportPrivateUsage]
                usage=usage, rate_limit_type=rate_limit_type
            )
        elif rate_limit_type == "output":
            total_tokens = rerank_usage[1]
        elif rate_limit_type == "input":
            total_tokens = rerank_usage[0]
        else:
            total_tokens = rerank_usage[0] + rerank_usage[1]
        reconcile_object = (
            rerank
            if input_value.get("rerank_tokens") is not None
            or input_value.get("billed_units_total_tokens") is not None
            else response
        )
        input_tokens, output_tokens, usage_resolved = limiter._resolve_io_token_reconcile_usage(  # pyright: ignore[reportPrivateUsage]
            reconcile_object
        )
        aggregate = limiter._aggregate_only_total_tokens(usage)  # pyright: ignore[reportPrivateUsage]
        combined = input_value.get("combined_usage_object")
        recovered = (0, 0, 0)
        if isinstance(combined, dict):
            recovered_source: Mapping[str, object] = {"combined_usage_object": typed_usage(combined)}
            recovered = limiter._recovered_partial_usage_tokens(recovered_source)  # pyright: ignore[reportPrivateUsage]
    return {
        "rate_limit_type": rate_limit_type,
        "total_tokens": total_tokens,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "usage_resolved": usage_resolved,
        "aggregate_only_total_tokens": aggregate,
        "recovered": [recovered[0], recovered[1], recovered[2]],
    }
