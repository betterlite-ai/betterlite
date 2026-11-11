import logging
from decimal import Decimal
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.litellm_core_utils import core_helpers
from litellm.litellm_core_utils.core_helpers import (
    add_missing_spend_metadata_to_litellm_metadata,  # pyright: ignore[reportUnknownVariableType]
    coerce_token_limit,
    drop_params_env_flag,
    drop_params_flag,
    filter_internal_params,  # pyright: ignore[reportUnknownVariableType]
    get_litellm_metadata_from_kwargs,  # pyright: ignore[reportUnknownVariableType]
    get_or_create_metadata_bucket,  # pyright: ignore[reportUnknownVariableType]
    get_provider_response_headers_from_hidden_params,
    independent_snapshot,  # pyright: ignore[reportUnknownVariableType]
    is_expected_client_error,
    map_finish_reason,
    normalize_drop_params,
    process_response_headers,  # pyright: ignore[reportUnknownVariableType]
    proxy_stamped_used_client_oauth_token,
    reconstruct_model_name,  # pyright: ignore[reportUnknownVariableType]
    safe_divide,
    safe_divide_seconds,
)
from litellm.litellm_core_utils.llm_request_utils import (
    get_proxy_server_request_headers,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from starlette.exceptions import HTTPException

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

_FINISH_REASON_MAP_NAME = "_FINISH_REASON_MAP"
_SET_PROVIDER_HEADERS_NAME = "set_provider_response_headers_in_hidden_params"
_FINISH_REASON_MAP = cast(
    "dict[str, object]", getattr(core_helpers, _FINISH_REASON_MAP_NAME)
)
_set_provider_headers = cast(
    "Callable[[object, dict[str, str]], None]",
    getattr(core_helpers, _SET_PROVIDER_HEADERS_NAME),
)
_LOGGER = logging.getLogger("oracle.call_helpers")


class _WarningCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.count = 0

    def emit(self, record: logging.LogRecord) -> None:
        self.count += 1 if record.levelno >= logging.WARNING else 0


class _FixtureError(Exception):
    pass


class _HiddenResponse:
    def __init__(self, hidden: Json) -> None:
        self._hidden_params: Json = hidden


def _object(value: Json | None, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def _string(value: Json | None, name: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def _optional_string(value: Json | None, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _list(value: Json | None, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return value


def _object_list(value: Json | None, name: str) -> list[Object]:
    return [_object(item, name) for item in _list(value, name)]


def _string_list(value: Json | None, name: str) -> list[str]:
    return [_string(item, name) for item in _list(value, name)]


def _string_map(value: Json | None, name: str) -> dict[str, str]:
    entries = _object(value, name)
    return {key: _string(entry, f"{name}.{key}") for key, entry in entries.items()}


def _number(value: Json | None, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} must be a number")
    return float(value)


def _decimal_text(value: float) -> str:
    return format(Decimal(str(value)).normalize(), "f")


def _fixture_error(fact: Object) -> BaseException:
    status = fact.get("status_code")
    initial: int = status if isinstance(status, int) and not isinstance(status, bool) else 400
    if fact.get("http_exception") is True:
        error: BaseException = HTTPException(status_code=initial)
    elif fact.get("base_llm_exception") is True:
        error = BaseLLMException(status_code=initial, message="fixture")
    else:
        error = _FixtureError()
    for name in ("code", "status_code", "llm_provider", "category"):
        setattr(error, name, fact.get(name))
    return error


def _finish_reasons(input_value: Object) -> Json:
    return cast(
        Json,
        [
            {
                "reason": map_finish_reason(raw),
                "recognized": raw in _FINISH_REASON_MAP,
            }
            for raw in _string_list(input_value.get("values"), "values")
        ],
    )


def _token_limits(input_value: Object) -> Json:
    return cast(
        Json,
        [
            coerce_token_limit(value)
            for value in _list(input_value.get("values"), "values")
        ],
    )


def _expected_client_error(input_value: Object) -> Json:
    return cast(
        Json,
        [
            is_expected_client_error(None)
            if raw is None
            else is_expected_client_error(_fixture_error(_object(raw, "fact")))
            for raw in _list(input_value.get("facts"), "facts")
        ],
    )


def _metadata_bucket(input_value: Object) -> Json:
    results: list[Json] = []
    for raw in _object_list(input_value.get("steps"), "steps"):
        request_data: dict[str, Json] = {}
        for name in ("metadata", "litellm_metadata"):
            if name in raw:
                request_data[name] = raw[name]
        key, bucket = cast(
            "tuple[str, dict[str, Json]]",
            get_or_create_metadata_bucket(request_data),
        )
        results.append({"name": key, "bucket": bucket})
    return results


def _trusted_step(raw: Json) -> Json:
    step = _object(raw, "trusted step")
    function = _string(step.get("function"), "function")
    if function == "litellm_metadata":
        kwargs: dict[str, Json] = {}
        params = step.get("litellm_params")
        if params is not None:
            kwargs["litellm_params"] = _object(params, "litellm_params")
        return cast(Json, get_litellm_metadata_from_kwargs(kwargs))
    if function == "stamped_oauth_token":
        metadata_raw = step.get("metadata")
        params_raw = step.get("litellm_params")
        metadata = None if metadata_raw is None else _object(metadata_raw, "metadata")
        params = None if params_raw is None else _object(params_raw, "litellm_params")
        return cast(Json, proxy_stamped_used_client_oauth_token(metadata, params))
    if function == "add_missing_spend_metadata":
        internal = dict(_object(step.get("litellm_metadata"), "litellm_metadata"))
        caller = _object(step.get("metadata"), "metadata")
        return cast(Json, add_missing_spend_metadata_to_litellm_metadata(internal, caller))
    raise ValueError(f"unknown trusted flags function: {function}")


def _trusted_flags(input_value: Object) -> Json:
    return [_trusted_step(raw) for raw in _list(input_value.get("steps"), "steps")]


def _model_reconstruction(input_value: Object) -> Json:
    results: list[Json] = []
    for entry in _object_list(input_value.get("values"), "values"):
        model = _string(entry.get("model"), "model")
        provider = _optional_string(entry.get("provider"), "provider")
        deployment = _optional_string(entry.get("deployment"), "deployment")
        metadata: dict[str, Json] = {} if deployment is None else {"deployment": deployment}
        results.append(reconstruct_model_name(model, provider, metadata))
    return results


def _header_step(raw: Json) -> Json:
    step = _object(raw, "header step")
    function = _string(step.get("function"), "function")
    if function == "project":
        headers = _string_map(step.get("headers"), "headers")
        return cast(Json, process_response_headers(headers))
    if function == "set_hidden":
        headers = _string_map(step.get("headers"), "headers")
        existing = _object(step.get("existing", {}), "existing")
        hidden: dict[str, Json] = {"additional_headers": existing}
        response = _HiddenResponse(hidden)
        _set_provider_headers(response, headers)
        return {
            "headers": hidden.get("headers"),
            "additional_headers": hidden.get("additional_headers"),
        }
    if function == "get_hidden":
        response = _HiddenResponse(step.get("hidden"))
        headers = get_provider_response_headers_from_hidden_params(response)
        return None if headers is None else cast(Json, dict(headers))
    raise ValueError(f"unknown response headers function: {function}")


def _response_headers(input_value: Object) -> Json:
    return [_header_step(raw) for raw in _list(input_value.get("steps"), "steps")]


def _upstream_internal_header(input_value: Object) -> Json:
    upstream = _string_map(input_value.get("upstream"), "upstream")
    trusted = _object(input_value.get("trusted"), "trusted")
    return {
        "upstream": cast(Json, process_response_headers(upstream)),
        "trusted": cast(
            Json,
            process_response_headers(trusted, preserve_litellm_internal_headers=True),
        ),
    }


def _snapshot_isolation(input_value: Object) -> Json:
    record = _object(input_value.get("record"), "record")
    return cast(Json, independent_snapshot(record))


def _filter_internal(input_value: Object) -> Json:
    record = _object(input_value.get("record"), "record")
    additional = input_value.get("additional", [])
    names: set[str] = set()
    if isinstance(additional, list):
        names = {name for name in additional if isinstance(name, str)}
    return cast(Json, filter_internal_params(record, names))


def _drop_env_flag(configured: str | None) -> Json:
    environ: dict[str, str] = (
        {} if configured is None else {"LITELLM_DROP_PARAMS": configured}
    )
    capture = _WarningCapture()
    _LOGGER.addHandler(capture)
    try:
        enabled = drop_params_env_flag(environ, _LOGGER)
    finally:
        _LOGGER.removeHandler(capture)
    return {"enabled": enabled, "warned": capture.count > 0}


def _drop_env_invalid(input_value: Object) -> Json:
    return [
        _drop_env_flag(_optional_string(value, "value"))
        for value in _list(input_value.get("values"), "values")
    ]


def _drop_request_flag(value: Json) -> Json:
    capture = _WarningCapture()
    _LOGGER.addHandler(capture)
    try:
        enabled = drop_params_flag(value, "call_helpers", _LOGGER)
    finally:
        _LOGGER.removeHandler(capture)
    return {
        "normalized": normalize_drop_params(value),
        "enabled": enabled,
        "warned": capture.count > 0,
    }


def _drop_request_invalid(input_value: Object) -> Json:
    return [
        _drop_request_flag(value)
        for value in _list(input_value.get("values"), "values")
    ]


def _safe_divide(input_value: Object) -> Json:
    return [
        _decimal_text(
            safe_divide(
                _number(pair.get("numerator"), "numerator"),
                _number(pair.get("denominator"), "denominator"),
                _number(pair.get("default"), "default"),
            )
        )
        for pair in _object_list(input_value.get("pairs"), "pairs")
    ]


def _safe_divide_seconds(input_value: Object) -> Json:
    results: list[Json] = []
    for pair in _object_list(input_value.get("pairs"), "pairs"):
        default_raw = pair.get("default")
        default = None if default_raw is None else _number(default_raw, "default")
        result = safe_divide_seconds(
            _number(pair.get("seconds"), "seconds"),
            _number(pair.get("denominator"), "denominator"),
            default,
        )
        results.append(None if result is None else _decimal_text(result))
    return results


def _request_headers(input_value: Object) -> Json:
    return cast(
        Json,
        [
            get_proxy_server_request_headers(
                None if raw is None else _object(raw, "params")
            )
            for raw in _list(input_value.get("steps"), "steps")
        ],
    )


def run(input_value: dict[str, Json]) -> Json:  # noqa: C901, PLR0911, PLR0912
    _ = litellm.__name__
    function = _string(input_value.get("function"), "function")
    if function == "finish_reasons":
        return _finish_reasons(input_value)
    if function == "token_limits":
        return _token_limits(input_value)
    if function == "expected_client_error":
        return _expected_client_error(input_value)
    if function == "metadata_bucket":
        return _metadata_bucket(input_value)
    if function == "trusted_flags":
        return _trusted_flags(input_value)
    if function == "model_reconstruction":
        return _model_reconstruction(input_value)
    if function == "response_headers":
        return _response_headers(input_value)
    if function == "upstream_internal_header":
        return _upstream_internal_header(input_value)
    if function == "snapshot_isolation":
        return _snapshot_isolation(input_value)
    if function == "filter_internal":
        return _filter_internal(input_value)
    if function == "drop_env_invalid":
        return _drop_env_invalid(input_value)
    if function == "drop_request_invalid":
        return _drop_request_invalid(input_value)
    if function == "safe_divide":
        return _safe_divide(input_value)
    if function == "safe_divide_seconds":
        return _safe_divide_seconds(input_value)
    if function == "request_headers":
        return _request_headers(input_value)
    raise ValueError(f"unknown call_helpers function: {function}")
