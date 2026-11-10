from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.llm_cost_calc.zero_cost_diagnostic import (
    diagnose_zero_cost,
    used_pricing_keys,
    zero_cost_warning,
)
from litellm.types.utils import Usage

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _optional_string(value: Json | None, name: str) -> str | None:
    return _string(value, name) if value is not None else None


def _run_probe(probe: dict[str, Json]) -> Json:
    usage = cast("Callable[..., Usage]", Usage)(**_object(probe.get("usage"), "usage"))
    pricing = _object(probe.get("pricing"), "pricing")
    calculation_failed = probe.get("calculation_failed") is True
    used_keys = list(used_pricing_keys(usage))
    diagnostic = diagnose_zero_cost(
        usage,
        _string(probe.get("pricing_model"), "pricing_model"),
        pricing,
        calculation_failed,
    )
    warning = None if diagnostic is None else zero_cost_warning(
        diagnostic,
        model_group=_optional_string(probe.get("model_group"), "model_group"),
        model=_string(probe.get("model"), "model"),
        custom_llm_provider=_optional_string(
            probe.get("custom_llm_provider"), "custom_llm_provider"
        ),
        usage=usage,
    )
    return {
        "used_keys": cast(Json, used_keys),
        "diagnostic": cast(Json, diagnostic),
        "warning": warning,
    }


def run(input_value: dict[str, Json]) -> Json:
    probes = input_value.get("probes")
    if not isinstance(probes, list):
        raise ValueError("probes must be a list")
    return [_run_probe(_object(probe, "probe")) for probe in probes]
