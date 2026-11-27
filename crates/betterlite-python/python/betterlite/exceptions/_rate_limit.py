import enum
from typing import Final

from pydantic import JsonValue


class RateLimitErrorCategory(str, enum.Enum):
    VENDOR_RATE_LIMIT = "vendor_rate_limit"
    VENDOR_BATCH_RATE_LIMIT = "vendor_batch_rate_limit"
    LITELLM_RATE_LIMIT = "litellm_rate_limit"
    LITELLM_BATCH_RATE_LIMIT = "litellm_batch_rate_limit"


class RateLimitType(str, enum.Enum):
    REQUESTS = "requests"
    TOKENS = "tokens"
    CONCURRENT_REQUESTS = "concurrent_requests"
    BUDGET = "budget"
    MAX_ITERATIONS = "max_iterations"


_RATE_LIMIT_CATEGORY_VALUES: Final[dict[str, str]] = {
    category.value: category.value for category in RateLimitErrorCategory
}
_RATE_LIMIT_TYPE_VALUES: Final[dict[str, str]] = {
    limit_type.value: limit_type.value for limit_type in RateLimitType
}


def _normalize_rate_limit_value(value: JsonValue, values: dict[str, str]) -> str | None:
    match value:
        case str() as text:
            return values.get(text)
        case bool() | int() | float() | list() | dict() | None:
            return None


def validate_rate_limit_category(value: JsonValue) -> str | None:
    return _normalize_rate_limit_value(
        value,
        _RATE_LIMIT_CATEGORY_VALUES,
    )


def validate_rate_limit_type(value: JsonValue) -> str | None:
    return _normalize_rate_limit_value(
        value,
        _RATE_LIMIT_TYPE_VALUES,
    )
