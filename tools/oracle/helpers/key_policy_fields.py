from datetime import UTC, datetime, tzinfo
from typing import Final, Literal, Self, TypedDict

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
    _set_key_rotation_fields,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L545-L579
    _update_policy_request,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L378-L391
    _validate_key_alias_format,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L7739-L7773
    _verification_token_from_row,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L361-L363
    handle_key_type,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L852-L867
    validate_model_max_budget,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L7809-L7840
)
from litellm.types.secret_managers.main import KeyManagementSettings
from pydantic import JsonValue, TypeAdapter
from pydantic_core import to_json
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)
_WINDOW: Final = TypeAdapter(dict[str, JsonValue])


# the case supplies the request time, so every LiteLLM datetime.now has one answer
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


def _alias_answer(call: AliasCall) -> JsonValue:
    litellm.key_alias_pattern = call["pattern"]
    litellm.enable_key_alias_format_validation = call["enabled"]
    _validate_key_alias_format(call["alias"])
    return None


def _rotation_answer(call: RotationCall) -> JsonValue:
    FrozenClock.fixed = datetime.fromisoformat(call["now"])
    data: dict[str, JsonValue] = dict(call["data"])
    settings = KeyManagementSettings(store_virtual_keys=call["store_virtual_keys"])
    litellm._key_management_settings = settings  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L1305-L1309
    key_management_endpoints.datetime = FrozenClock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L530-L542
    _set_key_rotation_fields(
        data,
        call["auto_rotate"],
        call["rotation_interval"],
        call["existing_alias"],
    )
    return _JSON.validate_json(to_json(data))


def _effective_policy_answer(call: EffectivePolicyCall) -> JsonValue:
    existing = _verification_token_from_row(call["existing"])
    request: UpdateKeyRequest | RegenerateKeyRequest
    if call["operation"] == "update":
        request = UpdateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1441
    else:
        request = RegenerateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1487
    record = _update_policy_request(call["operation"], existing, call["patch"], request)
    return _JSON.validate_json(record.model_dump_json())


def _regenerate_answer(call: RegenerateCall) -> JsonValue:
    request = RegenerateKeyRequest.model_validate(call["body"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1487
    patch_request = _regenerate_request_as_update_request(call["key"], request)
    if patch_request is None:
        return None
    return _JSON.validate_json(patch_request.model_dump_json())


def _budget_windows_answer(call: BudgetWindowsCall) -> JsonValue:
    FrozenClock.fixed = datetime.fromisoformat(call["now"])
    limits: JsonValue = _JSON.validate_python(call["budget_limits"])
    body: dict[str, JsonValue] = {"budget_limits": limits}
    data = GenerateKeyRequest.model_validate(body)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1376
    timezone_utils.datetime = FrozenClock  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/timezone_utils.py#L64-L71
    windows = _generate_budget_windows(data.budget_limits)
    if windows is None:
        return None
    rows: list[JsonValue] = [_WINDOW.validate_python(window) for window in windows]
    return rows


def _key_type_answer(call: KeyTypeCall) -> JsonValue:
    data = GenerateKeyRequest.model_validate(call["request"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L1376
    handled: JsonValue = _JSON.validate_python(handle_key_type(data, call["columns"]))  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/key_management_endpoints.py#L852-L867
    return handled


def answer(call: Call) -> JsonValue:
    result: JsonValue = None
    match call["function"]:
        case "alias":
            result = _alias_answer(call)
        case "rotation":
            result = _rotation_answer(call)
        case "effective_policy":
            result = _effective_policy_answer(call)
        case "model_budget":
            validate_model_max_budget(call["budgets"])
        case "json_column":
            result = _JSON.validate_python(
                _decode_json_string_column(call["column"], call["value"])
            )
        case "verification_row":
            token = _verification_token_from_row(call["row"])
            result = _JSON.validate_json(token.model_dump_json())
        case "key_type":
            result = _key_type_answer(call)
        case "regenerate":
            result = _regenerate_answer(call)
        case "budget_windows":
            result = _budget_windows_answer(call)
    return result


def call_outcome(call: Call) -> JsonValue:
    try:
        value = answer(call)
    except (ValueError, TypeError, KeyError, ProxyException):  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return {"ok": False}
    if call["function"] in ("alias", "model_budget"):
        return {"ok": True}
    return {"ok": True, "value": value}


def run(input_value: Input) -> JsonValue:
    outcomes: list[JsonValue] = [call_outcome(call) for call in input_value["calls"]]
    return _JSON.validate_python(outcomes)
