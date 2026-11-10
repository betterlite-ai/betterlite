from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.llms.vertex_ai.gemini import transformation
from litellm.llms.vertex_ai.gemini.transformation import (
    _gemini_convert_messages_with_history,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
)

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

litellm.suppress_debug_info = True


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
    recorder = _WarningRecorder()
    try:
        with patch.object(transformation, "verbose_logger", recorder):
            contents = _gemini_convert_messages_with_history(messages, model=model)
    except Exception as error:  # noqa: BLE001 - every conversion failure is part of the answer
        status = getattr(error, "status_code", None)
        error_type = getattr(error, "type", None)
        param = getattr(error, "param", None)
        return {
            "error": {
                "status": status if isinstance(status, int) else None,
                "type": error_type if isinstance(error_type, str) else None,
                "param": param if isinstance(param, str) else None,
                "code": str(status) if isinstance(status, int) else None,
            }
        }
    return {"contents": cast(Json, contents), "warnings": cast(Json, recorder.subjects)}


def run(input_value: dict[str, Json]) -> Json:
    cases = cast("list[Json]", input_value["cases"])
    return {"results": [_history(case) for case in cases]}
