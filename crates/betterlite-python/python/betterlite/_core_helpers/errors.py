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
def _is_provider_originated(exception: BaseException) -> bool:
    # ast-grep-ignore: no-isinstance
    if isinstance(exception, _LimitRejection) and _is_litellm_limit(exception.category):
        return False
    # ast-grep-ignore: no-isinstance
    if isinstance(exception, _ProviderError):
        return bool(exception.llm_provider)
    return False


def _is_litellm_limit(category: PythonValue) -> bool:
    name = text_of(category)
    return name is not None and name in _LITELLM_LIMIT_CATEGORIES


def _status_value(exception: BaseException) -> PythonValue:
    code = _code_of(exception)
    if code is not None:
        return code
    return _status_of(exception)


def _code_of(exception: BaseException) -> PythonValue:
    # ast-grep-ignore: no-isinstance
    if isinstance(exception, _CodeError):
        return exception.code
    return None


def _status_of(exception: BaseException) -> PythonValue:
    # ast-grep-ignore: no-isinstance
    if isinstance(exception, _StatusError):
        return exception.status_code
    return None


def _client_status(value: PythonValue) -> bool:
    match value:
        case bool():
            return False
        case int() as status:
            return _CLIENT_ERROR_MIN <= status < _CLIENT_ERROR_MAX
        case str() | float() as raw:
            return _status_from_text(raw)
        case object():
            return False


def _status_from_text(raw: str | float) -> bool:
    try:
        status = int(str(raw))
    except ValueError:
        return False
    return _CLIENT_ERROR_MIN <= status < _CLIENT_ERROR_MAX
