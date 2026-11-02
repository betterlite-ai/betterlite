from decimal import Decimal
from typing import TypeAlias, cast

from litellm.cost_calculator import (
    default_image_cost_calculator,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.litellm_core_utils.llm_cost_calc.utils import (
    calculate_image_response_cost_from_usage,
)
from litellm.llms.azure_ai.image_generation.cost_calculator import (
    cost_calculator as azure_cost_calculator,
)
from litellm.llms.openai.image_generation.cost_calculator import (
    cost_calculator as openai_cost_calculator,
)
from litellm.types.utils import (
    CompletionTokensDetailsWrapper,
    ImageObject,
    ImageResponse,
    ImageUsage,
    ModelInfo,
    PromptTokensDetailsWrapper,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _decimal_str(value: float) -> str:
    return format(Decimal(repr(value)).normalize(), "f")


def _model_info(input_value: dict[str, Json]) -> ModelInfo | None:
    model_info = input_value.get("model_info")
    return None if model_info is None else cast(ModelInfo, model_info)


def _image_objects(count: int) -> list[ImageObject]:
    return [ImageObject(url="https://example.invalid/generated.png") for _ in range(count)]


def _prompt_tokens_details(fields: Json) -> PromptTokensDetailsWrapper | None:
    if fields is None:
        return None
    details = cast("dict[str, Json]", fields)
    return PromptTokensDetailsWrapper.model_construct(
        text_tokens=cast("int | None", details.get("text_tokens")),
        audio_tokens=cast("int | None", details.get("audio_tokens")),
        image_tokens=cast("int | None", details.get("image_tokens")),
        video_tokens=cast("int | None", details.get("video_tokens")),
        web_search_requests=cast("int | None", details.get("web_search_requests")),
    )


def _completion_tokens_details(fields: Json) -> CompletionTokensDetailsWrapper | None:
    if fields is None:
        return None
    details = cast("dict[str, Json]", fields)
    return CompletionTokensDetailsWrapper.model_construct(
        text_tokens=cast("int | None", details.get("text_tokens")),
        audio_tokens=cast("int | None", details.get("audio_tokens")),
        image_tokens=cast("int | None", details.get("image_tokens")),
        video_tokens=cast("int | None", details.get("video_tokens")),
        reasoning_tokens=cast("int | None", details.get("reasoning_tokens")),
    )


def _chat_usage(usage_value: dict[str, Json]) -> Usage:
    prompt_tokens_details = _prompt_tokens_details(usage_value.get("prompt_details"))
    cache_read_tokens = usage_value.get("cache_read_tokens")
    if cache_read_tokens is not None:
        if prompt_tokens_details is None:
            prompt_tokens_details = PromptTokensDetailsWrapper.model_construct(
                cached_tokens=cast("int | None", cache_read_tokens)
            )
        else:
            prompt_tokens_details.cached_tokens = cast("int | None", cache_read_tokens)
    return Usage.model_construct(
        prompt_tokens=cast(int, usage_value.get("prompt_tokens", 0)),
        completion_tokens=cast(int, usage_value.get("completion_tokens", 0)),
        total_tokens=cast(int, usage_value.get("total_tokens", 0)),
        prompt_tokens_details=prompt_tokens_details,
        completion_tokens_details=_completion_tokens_details(usage_value.get("completion_details")),
    )


def _image_usage(details_value: dict[str, Json]) -> ImageUsage:
    return ImageUsage.model_construct(
        input_tokens=cast("int | None", details_value.get("input_tokens")),
        output_tokens=cast("int | None", details_value.get("output_tokens")),
        total_tokens=cast("int | None", details_value.get("total_tokens")),
        input_tokens_details=details_value.get("input_tokens_details"),
        completion_tokens_details=details_value.get("completion_tokens_details"),
        output_tokens_details=details_value.get("output_tokens_details"),
    )


def _usage_view(usage_value: dict[str, Json]) -> Usage | ImageUsage:
    kind = usage_value.get("kind")
    if kind == "chat":
        return _chat_usage(usage_value)
    if kind == "image":
        return _image_usage(usage_value)
    raise ValueError(f"unknown usage kind: {kind!r}")


def _run_default(input_value: dict[str, Json]) -> Json:
    cost = default_image_cost_calculator(
        model=cast(str, input_value["model"]),
        custom_llm_provider=cast("str | None", input_value.get("custom_llm_provider")),
        quality=cast("str | None", input_value.get("quality")),
        n=cast("int | None", input_value.get("n_images")),
        size=cast("str | None", input_value.get("size")),
        model_info=_model_info(input_value),
    )
    return {"cost": _decimal_str(cost)}


def _run_openai(input_value: dict[str, Json]) -> Json:
    image_count = cast(int, input_value.get("image_count", 1))
    usage_value = cast("dict[str, Json] | None", input_value.get("usage"))
    image_response = ImageResponse.model_construct(
        created=0,
        data=_image_objects(image_count),
        usage=None if usage_value is None else _usage_view(usage_value),
    )
    cost = openai_cost_calculator(
        model=cast(str, input_value["model"]),
        image_response=image_response,
        custom_llm_provider=cast("str | None", input_value.get("custom_llm_provider")),
        model_info=_model_info(input_value),
    )
    return {"cost": _decimal_str(cost)}


def _run_azure(input_value: dict[str, Json]) -> Json:
    billing = cast("dict[str, Json]", input_value["billing"])
    image_count = cast(int, billing.get("image_count", 0))
    usage_value = cast("dict[str, Json] | None", billing.get("usage"))
    image_response = ImageResponse.model_construct(
        created=0,
        data=_image_objects(image_count),
        usage=None if usage_value is None else _usage_view(usage_value),
        size=cast("str | None", billing.get("size")),
    )
    cost = azure_cost_calculator(
        model=cast(str, input_value["model"]),
        image_response=image_response,
        size=cast("str | None", input_value.get("size")),
        n=cast("int | None", input_value.get("n_images")),
        optional_params={"width": input_value.get("width"), "height": input_value.get("height")},
        model_info=_model_info(input_value),
    )
    return {"cost": _decimal_str(cost)}


def _run_usage(input_value: dict[str, Json]) -> Json:
    usage = _image_usage(cast("dict[str, Json]", input_value["usage"]))
    image_response = ImageResponse.model_construct(created=0, data=[], usage=usage)
    cost = calculate_image_response_cost_from_usage(
        model=cast(str, input_value["model"]),
        image_response=image_response,
        custom_llm_provider=cast(str, input_value.get("custom_llm_provider") or ""),
        model_info=_model_info(input_value),
    )
    return {"cost": None if cost is None else _decimal_str(cost)}


def run(input_value: dict[str, Json]) -> Json:
    operation = input_value["operation"]
    if operation == "default":
        return _run_default(input_value)
    if operation == "openai":
        return _run_openai(input_value)
    if operation == "azure":
        return _run_azure(input_value)
    if operation == "usage":
        return _run_usage(input_value)
    raise ValueError(f"unknown image_cost operation: {operation}")
