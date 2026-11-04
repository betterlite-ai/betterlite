from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm._logging import verbose_logger
from litellm.completion_extras.litellm_responses_transformation.transformation import (
    LiteLLMResponsesTransformationHandler,
    _input_file_from_file_value,  # pyright: ignore[reportPrivateUsage]
)
from litellm.types.llms.openai import ChatCompletionImageObject

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Handler = LiteLLMResponsesTransformationHandler


def _text_by_role(value: dict[str, Json]) -> Json:
    output = Handler()._convert_content_to_responses_format(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(str | list[object] | None, value["content"]), cast(str, value["role"])
    )
    return cast(Json, output)


def _content_passthrough(value: dict[str, Json]) -> Json:
    messages: list[str] = []
    def capture(message: str, *args: object, **_kwargs: object) -> None:
        rendered = message % args if args else message
        if rendered.startswith(("Chat provider:   ", "Chat provider: Final")):
            messages.append(rendered)

    with patch.object(verbose_logger, "debug", capture):
        output = Handler()._convert_content_to_responses_format(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast(list[object], value["items"]), cast(str, value["role"])
        )
    return cast(Json, {"value": output, "diagnostics": messages})


def _image_and_file(value: dict[str, Json]) -> Json:
    item = cast(dict[str, object], value["item"])
    if item.get("type") == "file":
        return cast(Json, _input_file_from_file_value(item.get("file")))  # pyright: ignore[reportPrivateUsage]
    image = Handler()._convert_content_to_responses_format_image(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(ChatCompletionImageObject, item), "user"
    )
    return cast(Json, dict(image))


def _flat_tool_choice(value: dict[str, Json]) -> Json:
    choice = Handler()._normalize_tool_choice_for_responses_api(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        cast(object, value["choice"])
    )
    return cast(Json, choice)


class Annotation:
    def __init__(self, kind: str, value: Json) -> None:
        self.value = value
        if kind == "model_dump":
            self.model_dump = self._serialize
        elif kind == "model_dump_and_dict":
            self.model_dump = self._serialize
            self.dict = self._raise
        elif kind == "legacy_dict":
            self.dict = self._serialize
        elif kind == "raises":
            self.model_dump = self._raise

    def _serialize(self) -> Json:
        return self.value

    def _raise(self) -> Json:
        raise ValueError(cast(str, self.value))

    def __repr__(self) -> str:
        return "annotation"


def _annotation_fallback(value: dict[str, Json]) -> Json:
    raw_items = value.get("annotations")
    items = cast(list[dict[str, Json]] | None, raw_items)
    annotation_values: list[object] = []
    for item in items or []:
        kind = cast(str, item["kind"])
        if kind in ("dict", "model_dump", "model_dump_and_dict", "legacy_dict"):
            if kind == "dict":
                annotation_values.append(cast(dict[str, object], item["value"]))
            else:
                annotation_values.append(Annotation(kind, item["value"]))
        elif kind == "raises":
            annotation_values.append(Annotation(kind, item["error"]))
        else:
            annotation_values.append(7)
    logs: list[str] = []
    def capture(message: str, *args: object, **_kwargs: object) -> None:
        logs.append(message % args if args else message)

    with patch.object(verbose_logger, "debug", capture):
        output = Handler._convert_annotations_to_chat_format(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            annotation_values if items is not None else None
        )
    return cast(Json, {"value": output, "diagnostics": logs})


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "text_by_role": _text_by_role,
    "content_passthrough": _content_passthrough,
    "image_and_file_fields": _image_and_file,
    "flat_tool_choice": _flat_tool_choice,
    "annotation_fallback": _annotation_fallback,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation not in HANDLERS:
        raise ValueError(f"unsupported operation {operation}")
    return HANDLERS[operation](input_value)
