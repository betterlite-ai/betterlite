from decimal import Decimal
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.cost_calculator import (
    _layered_ocr_pricing,  # pyright: ignore[reportPrivateUsage]
    ocr_batch_cost,
    ocr_cost,
)
from litellm.llms.base_llm.ocr.transformation import OCRResponse, OCRUsageInfo

if TYPE_CHECKING:
    from litellm.cost_calculator import OCRPricing
    from litellm.types.utils import ModelInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    # No arithmetic ran before this for a pass-through value, so repr still carries the
    # JSON literal's digits; a computed cost is kept to inputs whose float product is exact.
    return format(Decimal(repr(value)).normalize(), "f")


def _optional_decimal_str(value: float | None) -> Json:
    return None if value is None else _decimal_str(value)


def _dict_or_none(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("expected an object or null")
    return value


def _run_layering(input_value: dict[str, Json]) -> Json:
    sources_value = input_value.get("sources", [])
    if not isinstance(sources_value, list):
        raise ValueError("sources must be a JSON array")
    sources = [_dict_or_none(cast("Json", source)) for source in sources_value]
    pricing = _layered_ocr_pricing(*sources)
    return {
        "ocr_cost_per_page": _optional_decimal_str(pricing.get("ocr_cost_per_page")),
        "ocr_cost_per_credit": _optional_decimal_str(pricing.get("ocr_cost_per_credit")),
        "annotation_cost_per_page": _optional_decimal_str(pricing.get("annotation_cost_per_page")),
    }


def _applied_catalog(catalog_value: Json) -> dict[str, Json] | None:
    catalog = _dict_or_none(catalog_value)
    if catalog is None:
        return None
    key = catalog.get("key")
    entry = _dict_or_none(catalog.get("entry"))
    if not isinstance(key, str) or entry is None:
        raise ValueError("catalog must have a string key and an object entry")
    model_cost = cast("dict[str, Json]", litellm.model_cost)
    original = dict(model_cost)
    model_cost.clear()
    model_cost[key] = entry
    return original


def _restore_catalog(original: dict[str, Json] | None) -> None:
    if original is None:
        return
    model_cost = cast("dict[str, Json]", litellm.model_cost)
    model_cost.clear()
    model_cost.update(original)


def _usage_int(usage_value: dict[str, Json], key: str) -> int | None:
    value = usage_value.get(key)
    if value is not None and not isinstance(value, int):
        raise ValueError(f"usage.{key} must be a JSON integer or null")
    return value


def _run_sync(input_value: dict[str, Json]) -> Json:
    model = cast(str, input_value["model"])
    custom_llm_provider = cast("str | None", input_value.get("custom_llm_provider"))
    model_info = _dict_or_none(input_value.get("model_info"))

    usage_value = input_value["usage"]
    if not isinstance(usage_value, dict):
        raise ValueError("usage must be an object")
    credits_value = usage_value.get("credits")
    if credits_value is not None and not isinstance(credits_value, int | float):
        raise ValueError("usage.credits must be a JSON number or null")

    original = _applied_catalog(input_value.get("catalog"))
    try:
        response = OCRResponse(
            pages=[],
            model=model,
            usage_info=OCRUsageInfo(
                credits=None if credits_value is None else float(credits_value),
                pages_processed=_usage_int(usage_value, "pages_processed"),
                pages_processed_annotation=_usage_int(usage_value, "pages_processed_annotation"),
            ),
        )
        cost, _completion_cost = ocr_cost(
            model=model,
            custom_llm_provider=custom_llm_provider,
            response=response,
            model_info=cast("OCRPricing | None", model_info),
        )
    finally:
        _restore_catalog(original)
    return {"cost": _decimal_str(cost)}


def _run_batch(input_value: dict[str, Json]) -> Json:
    model = cast(str, input_value["model"])
    custom_llm_provider = cast("str | None", input_value.get("custom_llm_provider"))
    model_info = _dict_or_none(input_value.get("model_info"))

    usage_value = input_value["usage"]
    if not isinstance(usage_value, dict):
        raise ValueError("usage must be an object")

    original = _applied_catalog(input_value.get("catalog"))
    try:
        usage_info = OCRUsageInfo(
            pages_processed=_usage_int(usage_value, "pages_processed"),
            pages_processed_annotation=_usage_int(usage_value, "pages_processed_annotation"),
        )
        cost, _completion_cost = ocr_batch_cost(
            model=model,
            custom_llm_provider=custom_llm_provider,
            usage_info=usage_info,
            model_info=cast("ModelInfo | None", model_info),
        )
    finally:
        _restore_catalog(original)
    return {"cost": _decimal_str(cost)}


def run(input_value: dict[str, Json]) -> Json:
    kind = input_value.get("kind")
    if kind == "layering":
        return _run_layering(input_value)
    if kind == "sync":
        return _run_sync(input_value)
    if kind == "batch":
        return _run_batch(input_value)
    raise ValueError(f"unknown kind: {kind!r}")
