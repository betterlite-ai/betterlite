from collections.abc import Callable
from decimal import Decimal
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm.llms.openai.cost_calculation as openai_cost_calculation
from litellm import cost_calculator
from litellm.cost_calculator import (
    _per_second_pricing_cost,  # pyright: ignore[reportPrivateUsage]
    default_video_cost_calculator,
)
from litellm.litellm_core_utils.llm_cost_calc.utils import (
    _is_above_128k,  # pyright: ignore[reportPrivateUsage]
    select_cost_metric_for_model,
)
from litellm.llms.openai.cost_calculation import cost_per_second, video_generation_cost
from litellm.types.utils import ModelInfo, ModelInfoBase

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _dict_or_none(value: Json) -> dict[str, Json] | None:
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ValueError("expected an object or null")
    return value


def _fixture_model_info(catalog: dict[str, Json] | None) -> Callable[..., ModelInfoBase]:
    def lookup(model: str, **_: object) -> ModelInfoBase:
        if catalog is None:
            raise ValueError(f"model {model} was not found in the catalog")
        return cast(ModelInfoBase, catalog)

    return lookup


def _run_audio_cost_per_second(input_value: dict[str, Json]) -> Json:
    model = cast(str, input_value["model"])
    custom_llm_provider = cast("str | None", input_value.get("custom_llm_provider"))
    duration = cast("int | float", input_value["duration"])
    catalog = _dict_or_none(input_value.get("catalog"))

    with patch.object(openai_cost_calculation, "get_model_info", _fixture_model_info(catalog)):
        prompt_cost, completion_cost = cost_per_second(
            model=model, custom_llm_provider=custom_llm_provider, duration=float(duration)
        )
    return {
        "prompt_cost": _decimal_str(prompt_cost),
        "completion_cost": _decimal_str(completion_cost),
    }


def _run_wall_clock_per_second(input_value: dict[str, Json]) -> Json:
    model = cast(str, input_value["model"])
    custom_llm_provider = cast("str | None", input_value.get("custom_llm_provider"))
    response_time_ms = cast("int | float | None", input_value.get("response_time_ms"))
    catalog = _dict_or_none(input_value.get("catalog"))

    fixture = _fixture_model_info(catalog)
    with patch.object(cost_calculator, "_cached_get_model_info_helper", fixture):
        result = _per_second_pricing_cost(
            model=model,
            custom_llm_provider=custom_llm_provider,
            response_time_ms=None if response_time_ms is None else float(response_time_ms),
        )
    if result is None:
        return {"charged": False}
    prompt_cost, completion_cost = result
    return {
        "charged": True,
        "prompt_cost": _decimal_str(prompt_cost),
        "completion_cost": _decimal_str(completion_cost),
    }


def _run_video(input_value: dict[str, Json]) -> Json:
    calculator = cast(str, input_value["calculator"])
    model = cast(str, input_value["model"])
    duration_seconds = cast("int | float", input_value["duration_seconds"])
    custom_llm_provider = cast("str | None", input_value.get("custom_llm_provider"))
    model_info = _dict_or_none(input_value.get("model_info"))
    video_resolution = cast("str | None", input_value.get("video_resolution"))

    calculate = video_generation_cost if calculator == "openai" else default_video_cost_calculator
    cost = calculate(
        model=model,
        duration_seconds=float(duration_seconds),
        custom_llm_provider=custom_llm_provider,
        model_info=cast("ModelInfo | None", model_info),
        video_resolution=video_resolution,
    )
    return {"cost": _decimal_str(cost)}


def _run_character_metric(input_value: dict[str, Json]) -> Json:
    record = cast("dict[str, Json]", input_value["record"])
    above_128k_tokens = cast("list[Json]", input_value["above_128k_tokens"])

    metric = select_cost_metric_for_model(cast(ModelInfo, record))
    above_128k = [
        _is_above_128k(float(cast("int | float | str", tokens))) for tokens in above_128k_tokens
    ]
    return {"metric": metric, "above_128k": cast("list[Json]", above_128k)}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    if operation == "audio_cost_per_second":
        return _run_audio_cost_per_second(input_value)
    if operation == "wall_clock_per_second":
        return _run_wall_clock_per_second(input_value)
    if operation == "video":
        return _run_video(input_value)
    if operation == "character_metric":
        return _run_character_metric(input_value)
    raise ValueError(f"unknown duration_cost operation: {operation}")
