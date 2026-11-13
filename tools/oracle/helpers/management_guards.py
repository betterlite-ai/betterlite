from typing import TypeAlias, cast

from fastapi import HTTPException
from litellm.proxy import proxy_server
from litellm.proxy._types import KeyRequestBase, LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.management_endpoints import common_utils
from litellm.types.utils import BudgetConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# JSON has no non-finite numbers, so a case spells them and both sides read the same value
_AMOUNTS: dict[str, float] = {
    "nan": float("nan"),
    "inf": float("inf"),
    "-inf": float("-inf"),
    "infinity": float("inf"),
    "-infinity": float("-inf"),
}


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _optional_string(value: Json, name: str) -> str | None:
    return None if value is None else _string(value, name)


def _optional_int(value: Json, name: str) -> int | None:
    if value is None:
        return None
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def _optional_mapping(value: Json) -> dict[str, Json] | None:
    return None if value is None else _object(value, "metadata")


def _optional_list(value: Json) -> list[Json] | None:
    if value is None:
        return None
    if not isinstance(value, list):
        raise ValueError("allowed_passthrough_routes must be a list")
    return value


def _amount(value: Json) -> float | None:
    if value is None:
        return None
    if isinstance(value, str):
        amount = _AMOUNTS.get(value)
        if amount is None:
            raise ValueError("an amount is a number or one of nan, inf, -inf")
        return amount
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("an amount is a number or one of nan, inf, -inf")
    return float(value)


def _budget(raw: Json) -> BudgetConfig:
    fields = _object(raw, "budget")
    return BudgetConfig(
        max_budget=_amount(fields.get("max_budget")),
        budget_duration=_optional_string(fields.get("budget_duration"), "budget_duration"),
        tpm_limit=_optional_int(fields.get("tpm_limit"), "tpm_limit"),
        rpm_limit=_optional_int(fields.get("rpm_limit"), "rpm_limit"),
    )


def _auth(call: dict[str, Json]) -> UserAPIKeyAuth:
    user_id = _optional_string(call.get("user_id"), "user_id")
    user_role = _optional_string(call.get("user_role"), "user_role")
    return UserAPIKeyAuth(
        user_id=user_id,
        user_role=cast(LitellmUserRoles | None, user_role),
    )


def _entity(call: dict[str, Json]) -> str:
    entity = _optional_string(call.get("entity"), "entity")
    return "key" if entity is None else entity


def _passed(value: Json) -> dict[str, Json]:
    return {"ok": True, "status": None, "detail": None, "value": value}


def _rejected(status: int | None, detail: str, value: Json) -> dict[str, Json]:
    return {"ok": False, "status": status, "detail": detail, "value": value}


def _error_text(error: HTTPException) -> str:
    detail = cast(object, error.detail)
    if isinstance(detail, dict):
        message = cast(dict[str, object], detail).get("error")
        return message if isinstance(message, str) else ""
    return detail if isinstance(detail, str) else ""


def _team_model_max_budget(raw: Json) -> None:
    if raw is None:
        common_utils.validate_team_model_max_budget(None, premium_user=True)
        return
    configs = {name: _budget(value) for name, value in _object(raw, "model_max_budget").items()}
    common_utils.validate_team_model_max_budget(configs, premium_user=True)


def _update_field(call: dict[str, Json]) -> dict[str, Json]:
    updated_kv = _object(call.get("updated_kv"), "updated_kv")
    field_name = _string(call.get("field_name"), "field_name")
    update = common_utils._update_metadata_field  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
    try:
        update(updated_kv, field_name)
    except TypeError:
        return _rejected(None, "", updated_kv)
    return _passed(updated_kv)


def _update_fields(call: dict[str, Json]) -> dict[str, Json]:
    updated_kv = _object(call.get("updated_kv"), "updated_kv")
    update = common_utils._update_metadata_fields  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
    try:
        update(updated_kv)
    except TypeError:
        return _rejected(None, "", updated_kv)
    return _passed(updated_kv)


def _guard(call: dict[str, Json], function: str) -> Json:
    if function == "validate_finite_spend":
        common_utils.validate_finite_spend(_amount(call.get("spend")))
        return None
    if function == "validate_budget_duration":
        status_code = _optional_int(call.get("status_code"), "status_code")
        common_utils.validate_budget_duration(
            _optional_string(call.get("budget_duration"), "budget_duration"),
            400 if status_code is None else status_code,
        )
        return None
    if function == "validate_team_model_max_budget":
        _team_model_max_budget(call.get("model_max_budget"))
        return None
    if function == "caller_user_id_for_non_admin":
        return common_utils.require_caller_user_id_for_non_admin(_auth(call))
    return _permission(call, function)


def _permission(call: dict[str, Json], function: str) -> Json:
    if function == "check_passthrough_routes":
        data = KeyRequestBase(
            allowed_passthrough_routes=_optional_list(call.get("allowed_passthrough_routes")),
            metadata=_optional_mapping(call.get("metadata")),
        )
        check = common_utils._check_passthrough_routes_caller_permission  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        check(data, _auth(call), entity=_entity(call))
        return None
    if function == "check_guardrail_opt_out":
        check = common_utils._check_disable_global_guardrails_caller_permission  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        check(
            _optional_bool(call.get("disable_global_guardrails")),
            _optional_mapping(call.get("metadata")),
            _auth(call),
            entity=_entity(call),
            existing_metadata=_optional_mapping(call.get("existing_metadata")),
        )
        return None
    if function == "set_object_metadata_field":
        data = KeyRequestBase(metadata=_optional_mapping(call.get("metadata")))
        set_field = common_utils._set_object_metadata_field  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        set_field(data, _string(call.get("field_name"), "field_name"), call.get("value"))
        return cast(Json, data.metadata)
    raise ValueError(f"unknown management guard: {function}")


def _optional_bool(value: Json) -> bool | None:
    if value is None:
        return None
    if not isinstance(value, bool):
        raise ValueError("disable_global_guardrails must be a boolean")
    return value


def _execute(call: dict[str, Json]) -> dict[str, Json]:
    function = _string(call.get("function"), "function")
    if function == "update_metadata_field":
        return _update_field(call)
    if function == "update_metadata_fields":
        return _update_fields(call)
    try:
        return _passed(_guard(call, function))
    except HTTPException as error:
        return _rejected(error.status_code, _error_text(error), None)


def run(input_value: dict[str, Json]) -> Json:
    proxy_server.premium_user = True
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_execute(_object(call, "call")) for call in calls]
