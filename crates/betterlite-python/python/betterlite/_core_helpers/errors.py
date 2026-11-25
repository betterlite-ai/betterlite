from typing import Final, Protocol, runtime_checkable

from betterlite._core_helpers.values import PythonValue, text_of
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
def _is_provider_originated(
    exception: BaseException | _LimitRejection | _ProviderError,
) -> bool:
    # ast-grep-ignore: no-isinstance
    is_limiter = isinstance(exception, _LimitRejection) and _is_litellm_limit(
        exception.category
    )
    if is_limiter:
        return False
    # ast-grep-ignore: no-isinstance
    if isinstance(exception, _ProviderError):
        return bool(exception.llm_provider)
    return False


def _is_litellm_limit(category: PythonValue) -> bool:
    name = text_of(category)
    return name is not None and name in _LITELLM_LIMIT_CATEGORIES


def _status_value(
    exception: BaseException | _CodeError | _StatusError,
) -> PythonValue:
    match exception:
        case _CodeError():
            return exception.code
        case _StatusError():
            return exception.status_code
        case BaseException():
            return None


def _client_status(value: PythonValue) -> bool:
    # ast-grep-ignore: no-isinstance
    if isinstance(value, bool):
        return False
    # ast-grep-ignore: no-isinstance
    if isinstance(value, int):
        return _CLIENT_ERROR_MIN <= value < _CLIENT_ERROR_MAX
    # ast-grep-ignore: no-isinstance
    if isinstance(value, str | float):
        return _status_from_text(value)
    return False


def _status_from_text(raw: str | float) -> bool:
    try:
        status = int(str(raw))
    except ValueError:
        return False
    return _CLIENT_ERROR_MIN <= status < _CLIENT_ERROR_MAX
