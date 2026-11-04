from types import SimpleNamespace
from typing import TypeAlias, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig,
)
from litellm.types.utils import ChatCompletionAnnotation, Choices, Usage
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def _model_json(value: BaseModel) -> Json:
    return cast("Json", value.model_dump(mode="json"))


def run(input_value: dict[str, Json]) -> Json:
    images = input_value.get("images", [])
    choice = cast(
        "Choices",
        SimpleNamespace(
            message=SimpleNamespace(images=images),
            finish_reason=input_value.get("finish_reason"),
        ),
    )
    image_items = LiteLLMCompletionResponsesConfig._extract_image_generation_output_items(choice)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    annotations = (
        LiteLLMCompletionResponsesConfig._transform_chat_completion_annotations_to_response_output_annotations(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("list[ChatCompletionAnnotation] | None", input_value.get("annotations"))
        )
    )
    raw_usage = input_value.get("usage")
    usage = Usage.model_validate(raw_usage) if isinstance(raw_usage, dict) else Usage()
    projected_usage = (
        LiteLLMCompletionResponsesConfig._transform_chat_completion_usage_to_responses_usage(usage)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    status = LiteLLMCompletionResponsesConfig._map_finish_reason_to_image_generation_status(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast("str | None", input_value.get("finish_reason"))
    )
    return {
        "images": [_model_json(item) for item in image_items],
        "status": status,
        "annotations": [_model_json(item) for item in annotations],
        "usage": _model_json(projected_usage),
    }
