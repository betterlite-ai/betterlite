from datetime import UTC, datetime, tzinfo
from typing import Final, Literal, Self, TypedDict, override
from unittest.mock import patch

import litellm
from litellm.proxy._types import (
    GenerateKeyRequest,
    ProxyException,
    RegenerateKeyRequest,
    UpdateKeyRequest,
)
from litellm.proxy.common_utils import timezone_utils
from litellm.proxy.management_endpoints import key_management_endpoints
from litellm.proxy.management_endpoints.key_management_endpoints import (
    _decode_json_string_column,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L355-L358
    _generate_budget_windows,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L394-L407
    _regenerate_request_as_update_request,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L450-L462
    _set_key_rotation_fields,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L545-L579
    _update_policy_request,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L378-L391
    _validate_key_alias_format,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L7739-L7773
    _verification_token_from_row,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L361-L363
    handle_key_type,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L852-L867
    validate_model_max_budget,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L7809-L7840
)
from litellm.types.secret_managers.main import KeyManagementSettings
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_DATETIME: Final = TypeAdapter(datetime)


class FrozenClock(datetime):
    fixed: datetime = datetime(1970, 1, 1, tzinfo=UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = cls.fixed if tz is None else cls.fixed.astimezone(tz)
        return cls.fromtimestamp(moment.timestamp(), tz=moment.tzinfo)


class AliasCall(TypedDict):
    function: Literal["alias"]
    pattern: str | None
    enabled: bool
    alias: str | None


class RotationCall(TypedDict):
    function: Literal["rotation"]
    data: dict[str, JsonValue]
    auto_rotate: bool
    rotation_interval: str | None
    existing_alias: str | None
    store_virtual_keys: bool
    now: str


class EffectivePolicyCall(TypedDict):
    function: Literal["effective_policy"]
    operation: Literal["update", "regenerate"]
    existing: dict[str, JsonValue]
    patch: dict[str, JsonValue]
    request: dict[str, JsonValue]


class ModelBudgetCall(TypedDict):
    function: Literal["model_budget"]
    budgets: dict[str, JsonValue] | None


class JsonColumnCall(TypedDict):
    function: Literal["json_column"]
    column: str
    value: JsonValue


class VerificationRowCall(TypedDict):
    function: Literal["verification_row"]
    row: dict[str, JsonValue]


class KeyTypeCall(TypedDict):
    function: Literal["key_type"]
    request: dict[str, JsonValue]
    columns: dict[str, JsonValue]


class RegenerateCall(TypedDict):
    function: Literal["regenerate"]
    key: str
    body: dict[str, JsonValue]


class BudgetWindowsCall(TypedDict):
    function: Literal["budget_windows"]
    budget_limits: list[dict[str, JsonValue]] | None
    now: str


Call = (
    AliasCall
    | RotationCall
    | EffectivePolicyCall
    | ModelBudgetCall
    | JsonColumnCall
    | VerificationRowCall
    | KeyTypeCall
    | RegenerateCall
    | BudgetWindowsCall
)


class Input(TypedDict):
    calls: list[Call]


def _instant(text: str) -> datetime:
    return datetime.fromisoformat(text)


def _json_value(value: object) -> JsonValue:
    if isinstance(value, datetime):
        return _JSON.validate_json(_DATETIME.dump_json(value))
    return _JSON.validate_python(value)


def _alias_answer(call: AliasCall) -> JsonValue:
    with (
        patch.object(litellm, "key_alias_pattern", call["pattern"]),
        patch.object(litellm, "enable_key_alias_format_validation", call["enabled"]),
    ):
        _validate_key_alias_format(call["alias"])
    return None


def _model_budget_answer(call: ModelBudgetCall) -> JsonValue:
    validate_model_max_budget(call["budgets"])
    return None


def _json_column_answer(call: JsonColumnCall) -> JsonValue:
    return _JSON.validate_python(_decode_json_string_column(call["column"], call["value"]))


def _verification_row_answer(call: VerificationRowCall) -> JsonValue:
    token = _verification_token_from_row(call["row"])
    return _JSON.validate_json(token.model_dump_json())


def _rotation_answer(call: RotationCall) -> JsonValue:
    FrozenClock.fixed = _instant(call["now"])
    data: dict[str, JsonValue] = dict(call["data"])
    settings = KeyManagementSettings(store_virtual_keys=call["store_virtual_keys"])
    with (
        patch.object(key_management_endpoints, "datetime", FrozenClock),
        patch.object(litellm, "_key_management_settings", settings),
    ):
        _set_key_rotation_fields(
            data,
            call["auto_rotate"],
            call["rotation_interval"],
            call["existing_alias"],
        )
    stamped: dict[str, JsonValue] = {key: _json_value(value) for key, value in data.items()}
    return _JSON.validate_python(stamped)


def _effective_policy_answer(call: EffectivePolicyCall) -> JsonValue:
    existing = _verification_token_from_row(call["existing"])
    request: UpdateKeyRequest | RegenerateKeyRequest
    if call["operation"] == "update":
        request = UpdateKeyRequest.model_validate(call["request"])
    else:
        request = RegenerateKeyRequest.model_validate(call["request"])
    record = _update_policy_request(call["operation"], existing, call["patch"], request)
    return _JSON.validate_json(record.model_dump_json())


def _regenerate_answer(call: RegenerateCall) -> JsonValue:
    request = RegenerateKeyRequest.model_validate(call["body"])
    patch_request = _regenerate_request_as_update_request(call["key"], request)
    if patch_request is None:
        return None
    return _JSON.validate_json(patch_request.model_dump_json())


def _key_type_answer(call: KeyTypeCall) -> JsonValue:
    data = GenerateKeyRequest.model_validate(call["request"])
    handled: dict[str, JsonValue] = handle_key_type(data, call["columns"])
    return _JSON.validate_python(handled)


def _budget_windows_answer(call: BudgetWindowsCall) -> JsonValue:
    FrozenClock.fixed = _instant(call["now"])
    data = GenerateKeyRequest.model_validate({"budget_limits": call["budget_limits"]})
    with patch.object(timezone_utils, "datetime", FrozenClock):
        windows = _generate_budget_windows(data.budget_limits)
    if windows is None:
        return None
    return _JSON.validate_python([dict(window) for window in windows])


def answer(call: Call) -> JsonValue:
    match call["function"]:
        case "alias":
            result = _alias_answer(call)
        case "model_budget":
            result = _model_budget_answer(call)
        case "json_column":
            result = _json_column_answer(call)
        case "verification_row":
            result = _verification_row_answer(call)
        case "rotation":
            result = _rotation_answer(call)
        case "effective_policy":
            result = _effective_policy_answer(call)
        case "regenerate":
            result = _regenerate_answer(call)
        case "key_type":
            result = _key_type_answer(call)
        case "budget_windows":
            result = _budget_windows_answer(call)
    return result


def call_outcome(call: Call) -> JsonValue:
    try:
        value = answer(call)
    except (ValueError, TypeError, KeyError, ProxyException):
        return _JSON.validate_python({"ok": False})
    if call["function"] in ("alias", "model_budget"):
        return _JSON.validate_python({"ok": True})
    return _JSON.validate_python({"ok": True, "value": value})


def run(input_value: Input) -> JsonValue:
    outcomes = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
