from decimal import Decimal
from typing import TypeAlias, cast

from litellm.litellm_core_utils.ptu_pricing import (
    azure_spillover,
    declares_ptu,
    ptu_config_error,
    ptu_identity_error,
    ptu_terms,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _operation(input_value: dict[str, Json]) -> str:
    value = input_value.get("operation")
    if not isinstance(value, str):
        raise TypeError("operation must be a string")
    return value


def _model_info(input_value: dict[str, Json]) -> dict[str, object]:
    value = input_value.get("model_info")
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise TypeError("model_info must be an object")
    return cast(dict[str, object], value)


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else cast(str, value)


def _headers(value: Json) -> dict[str, object] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise TypeError("headers must be an object")
    return cast(dict[str, object], value)


def _decimal_text(value: float) -> str:
    # No arithmetic runs before this, so repr still carries the JSON literal's digits.
    return format(Decimal(repr(value)).normalize(), "f")


def _terms(model_info: dict[str, object]) -> Json:
    terms = ptu_terms(model_info)
    if terms is None:
        return None
    effective_to = terms.effective_to
    return {
        "team_id": terms.team_id,
        "ptu_count": terms.ptu_count,
        "cost_per_ptu_per_hour": _decimal_text(terms.cost_per_ptu_per_hour),
        "effective_from": terms.effective_from.isoformat(),
        "effective_to": None if effective_to is None else effective_to.isoformat(),
    }


def _spillover(input_value: dict[str, Json]) -> Json:
    spillover = azure_spillover(
        response_headers=_headers(input_value.get("response_headers")),
        additional_headers=_headers(input_value.get("additional_headers")),
    )
    if spillover is None:
        return None
    return {"from_deployment": spillover["from_deployment"]}


def run(input_value: dict[str, Json]) -> Json:
    operation = _operation(input_value)
    model_info = _model_info(input_value)
    if operation == "terms":
        return _terms(model_info)
    if operation == "config_error":
        return ptu_config_error(
            model_info, model_name=_optional_string(input_value, "model_name")
        )
    if operation == "identity":
        return ptu_identity_error(
            declared_id=_optional_string(input_value, "declared_id"),
            taken=input_value.get("taken") is True,
            current_id=_optional_string(input_value, "current_id"),
            model_name=_optional_string(input_value, "model_name"),
        )
    if operation == "declares":
        return declares_ptu(model_info)
    if operation == "spillover":
        return _spillover(input_value)
    raise ValueError(f"unknown ptu_pricing operation: {operation}")
