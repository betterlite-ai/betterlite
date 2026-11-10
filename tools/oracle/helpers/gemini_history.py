from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.prompt_templates import factory
from litellm.llms.vertex_ai.gemini import transformation
from litellm.llms.vertex_ai.gemini.transformation import (
    _gemini_convert_messages_with_history,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]

litellm.suppress_debug_info = True


class _MappedConvert:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


class _WarningRecorder:
    def __init__(self) -> None:
        self.subjects: list[str] = []

    def warning(self, message: object, *_args: object, **_kwargs: object) -> None:
        text = str(message)
        if "No text in user content" in text:
            self.subjects.append("blank_text")
        elif "No contents in messages" in text:
            self.subjects.append("empty_history")


def _history(case: Json) -> Json:
    fields = cast("dict[str, Json]", case)
    messages = cast("list[AllMessageValues]", fields.get("messages") or [])
    model = cast("str | None", fields.get("model"))
    mapping = cast("dict[str, str]", fields.get("data_urls") or {})
    recorder = _WarningRecorder()
    try:
        with (
            patch.object(transformation, "verbose_logger", recorder),
            patch.object(factory, "convert_url_to_base64", _MappedConvert(mapping)),
        ):
            contents = _gemini_convert_messages_with_history(messages, model=model)
    except Exception as error:  # noqa: BLE001 - every conversion failure is part of the answer
        status = getattr(error, "status_code", None)
        code = str(status) if isinstance(status, int) and not isinstance(status, bool) else None
        return cast(
            Json,
            {
                "error": {
                    "status": status if code is not None else None,
                    "type": None,
                    "param": None,
                    "code": code,
                }
            },
        )
    return cast(
        Json,
        {"contents": cast(Json, contents), "warnings": cast(Json, recorder.subjects)},
    )


def run(input_value: dict[str, Json]) -> Json:
    cases = cast("list[Json]", input_value["cases"])
    return {"results": [_history(case) for case in cases]}
