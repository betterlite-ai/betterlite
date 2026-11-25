import logging
import re
from collections.abc import Iterable, Mapping
from datetime import datetime
from typing import Final

from pydantic import TypeAdapter, ValidationError

from betterlite._core_helpers.values import CallableLeaf, PythonValue, text_of
from betterlite.types.utils.base import Dumpable

_CODEX_PREFIX: Final = re.compile("^codex[-_ /]", re.IGNORECASE)
_DROP_PARAMS_BOOL: Final[TypeAdapter[bool]] = TypeAdapter(bool)
DROP_PARAMS_ENV_VAR: Final = "LITELLM_DROP_PARAMS"


def is_codex_user_agent(user_agent: str) -> bool:
    matched = _CODEX_PREFIX.match(user_agent)
    return bool(matched)


def safe_divide_seconds(
    seconds: float, denominator: float, default: float | None = None
) -> float | None:
    if denominator <= 0:
        return default
    return float(seconds / denominator)


def normalize_drop_params(value: PythonValue) -> bool | None:
    if value is None:
        return None
    name = text_of(value)
    candidate: PythonValue = value if name is None else name.strip()
    try:
        return _DROP_PARAMS_BOOL.validate_python(candidate)
    except ValidationError:
        return None


def drop_params_flag(value: PythonValue, source: str, logger: logging.Logger) -> bool:
    normalized = normalize_drop_params(value)
    if normalized is None and value is not None:
        logger.warning("%s=%r is not a flag value, treating it as off", source, value)
    return bool(normalized)


def drop_params_env_flag(environ: Mapping[str, str], logger: logging.Logger) -> bool:
    raw = environ.get(DROP_PARAMS_ENV_VAR)
    configured = "" if raw is None else raw.strip()
    if len(configured) == 0:
        return False
    normalized = normalize_drop_params(configured)
    if normalized is None:
        logger.warning(
            "%s=%r is not a flag value, treating it as on. Set it to true or false",
            DROP_PARAMS_ENV_VAR,
            configured,
        )
        return True
    return normalized


def safe_divide(numerator: float, denominator: float, default: float = 0) -> int | float:
    if denominator == 0:
        return default
    return numerator / denominator


def coerce_token_limit(value: PythonValue) -> int | None:
    match value:
        case bool():
            return None
        case int() as limit:
            return limit
        case str() | float() as raw:
            try:
                return int(raw)
            except (TypeError, ValueError, OverflowError):
                return None
        case BaseException() | datetime() | Dumpable() | Iterable() | CallableLeaf() | None:
            return None
