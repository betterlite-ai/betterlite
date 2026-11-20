from typing import Literal, NotRequired, TypedDict

from pydantic import JsonValue


class PayloadFields(TypedDict):
    response_cost: NotRequired[float]
    total_tokens: NotRequired[int]
    prompt_tokens: NotRequired[int]
    completion_tokens: NotRequired[int]
    cache_hit: NotRequired[bool | None]
    usage_object: NotRequired[dict[str, JsonValue] | None]


class Input(PayloadFields):
    function: Literal[
        "request_and_spend",
        "zero_cost",
        "token_metrics",
        "token_detail",
        "media",
        "cache",
        "provider_cache_tokens",
        "cache_write_tokens",
    ]
    values: NotRequired[dict[str, str | None]]
    supported_labels: NotRequired[dict[str, list[str]]]
    provider_usage: NotRequired[dict[str, JsonValue]]
    prompt_details: NotRequired[dict[str, JsonValue]]
