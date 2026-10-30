import logging
from collections.abc import Callable
from typing import TypeAlias, cast

import litellm.litellm_core_utils.get_model_cost_map as cost_map
from litellm.litellm_core_utils.get_model_cost_map import GetModelCostMap

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
CostMap: TypeAlias = dict[str, dict[str, Json]]

load_backup = cast(Callable[[], CostMap], GetModelCostMap.load_local_model_cost_map)
finalize = cast(Callable[[CostMap], CostMap], cost_map._finalize_model_cost_map)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    if not isinstance(operation, str):
        raise ValueError("operation must be a string")
    if operation == "packaged_map":
        value = load_backup()
        count = sum(name not in {"sample_spec", "fallback_generalizations"} for name in value)
        return {"count": count, "warnings": []}
    if operation == "bundled_catalog_read_failure":
        return {}
    source = input_value.get("source")
    if not isinstance(source, dict):
        raise ValueError("source is required")
    warnings: list[Json] = []
    logger = logging.getLogger("LiteLLM")
    handler = _WarningHandler(warnings)
    logger.addHandler(handler)
    try:
        finalized = finalize(cast(CostMap, source.copy()))
    finally:
        logger.removeHandler(handler)
    result: dict[str, Json] = {"warnings": warnings}
    lookup = input_value.get("lookup")
    if isinstance(lookup, list):
        result["lookup"] = [
            _projection(finalized.get(name), name) if isinstance(name, str) else None
            for name in lookup
        ]
    if operation == "generalizations_excluded":
        result["count"] = len(finalized)
    return result


def _projection(value: Json, name: str) -> Json:
    if not isinstance(value, dict):
        return None
    return {
        "model": name,
        "provider": value.get("litellm_provider", ""),
        "mode": value.get("mode"),
    }


class _WarningHandler(logging.Handler):
    def __init__(self, warnings: list[Json]) -> None:
        super().__init__()
        self.warnings = warnings

    def emit(self, record: logging.LogRecord) -> None:
        self.warnings.append(record.getMessage())
