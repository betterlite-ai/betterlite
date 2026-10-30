import logging
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import litellm.litellm_core_utils.get_model_cost_map as cost_map
from litellm import utils
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
        return _bundled_failure()
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
    result: dict[str, Json] = {"warnings": warnings, "warning_count": len(warnings)}
    lookup = input_value.get("lookup")
    if isinstance(lookup, list):
        result["lookup"] = [
            _projection(finalized.get(name), name, finalized) if isinstance(name, str) else None
            for name in lookup
        ]
    if operation == "generalizations_excluded":
        result["count"] = len(finalized)
    return result


def _projection(value: Json, name: str, entries: CostMap) -> Json:
    if not isinstance(value, dict):
        return None
    canonical = next(
        (candidate for candidate, entry in entries.items() if entry is value),
        name,
    )
    return {
        "model": canonical,
        "provider": value.get("litellm_provider", ""),
        "mode": value.get("mode"),
    }


class _WarningHandler(logging.Handler):
    def __init__(self, warnings: list[Json]) -> None:
        super().__init__()
        self.warnings = warnings

    def emit(self, record: logging.LogRecord) -> None:
        self.warnings.append(record.getMessage())


class _CachedLoader(Protocol):
    def __call__(self) -> CostMap: ...
    def cache_clear(self) -> None: ...


def _bundled_failure() -> Json:
    load = cast(_CachedLoader, utils._get_bundled_model_cost_map)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    load.cache_clear()
    warnings: list[Json] = []
    logger = logging.getLogger("LiteLLM")
    handler = _WarningHandler(warnings)
    logger.addHandler(handler)
    try:
        with patch("litellm.utils.resources.files", side_effect=OSError("missing asset")) as asset:
            first = load()
            second = load()
            return {
                "catalog": cast(Json, first),
                "count": len(first),
                "cached": first is second,
                "reads": asset.call_count,
                "warnings": warnings,
            }
    finally:
        load.cache_clear()
        logger.removeHandler(handler)
