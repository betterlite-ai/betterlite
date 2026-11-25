from typing import Final, Protocol, runtime_checkable

from betterlite._core_helpers.values import PythonValue
from betterlite.exceptions._rate_limit import RateLimitErrorCategory

_LITELLM_LIMIT_CATEGORIES: Final = frozenset(
    {
        RateLimitErrorCategory.LITELLM_RATE_LIMIT.value,
        RateLimitErrorCategory.LITELLM_BATCH_RATE_LIMIT.value,
    }
)
_CLIENT_ERROR_MIN: Final = 400
_CLIENT_ERROR_MAX: Final = 500


@runtime_checkable
class _LimitRejection(Protocol):
    category: PythonValue


@runtime_checkable
class _ProviderError(Protocol):
    llm_provider: PythonValue


@runtime_checkable
class _CodeError(Protocol):
    code: PythonValue


@runtime_checkable
class _StatusError(Protocol):
    status_code: PythonValue


def is_expected_client_error(exception: BaseException | None) -> bool:
    if exception is None:
        return False
    if _is_provider_originated(exception):
        return False
    return _client_status(_status_value(exception))


# LiteLLM's order: a litellm limiter rejection is the proxy's, whatever provider it names.
def _is_provider_originated(exception: BaseException) -> bool:
    match exception:
        case _LimitRejection() if _is_litellm_limit(exception.category):
            return False
        case _ProviderError() if exception.llm_provider:
            return True
    return False


def _is_litellm_limit(category: PythonValue) -> bool:
    match category:
        case str() as name:
            return name in _LITELLM_LIMIT_CATEGORIES
    return False


def _status_value(exception: BaseException) -> PythonValue:
    match exception:
        case _CodeError() as error if error.code is not None:
            return error.code
    match exception:
        case _StatusError() as error:
            return error.status_code
    return None


def _client_status(value: PythonValue) -> bool:
    match value:
        case bool():
            return False
        case int() as status:
            return _CLIENT_ERROR_MIN <= status < _CLIENT_ERROR_MAX
        case str() | float() as raw:
            return _status_from_text(raw)
    return False


def _status_from_text(raw: str | float) -> bool:
    try:
        status = int(str(raw))
    except ValueError:
        return False
    return _CLIENT_ERROR_MIN <= status < _CLIENT_ERROR_MAX
