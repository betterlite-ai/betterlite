from types import SimpleNamespace
from typing import TYPE_CHECKING, Final, Literal, TypeAlias, cast

from litellm import utils as litellm_utils
from litellm.types import utils as types_utils

if TYPE_CHECKING:
    from litellm.types.utils import ModelResponse, ModelResponseStream

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
PromptCallType: TypeAlias = Literal[
    "acompletion",
    "completion",
    "text_completion",
    "embedding",
    "image_generation",
    "audio_transcription",
    "moderation",
]
_PROMPT_CALL_TYPES: Final[dict[str, PromptCallType]] = {
    "acompletion": "acompletion",
    "completion": "completion",
    "text_completion": "text_completion",
    "embedding": "embedding",
    "image_generation": "image_generation",
    "audio_transcription": "audio_transcription",
    "moderation": "moderation",
}


def _object(value: Json | None, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


def _string(value: Json | None, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _list(value: Json | None, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def _prompt_call_type(name: str) -> PromptCallType:
    call_type = _PROMPT_CALL_TYPES.get(name)
    if call_type is None:
        raise ValueError(f"unknown call type {name}")
    return call_type


def _prompt_data(raw: dict[str, Json]) -> dict[str, object]:
    data: dict[str, object] = {}
    for name in ("messages", "prompt", "input"):
        value = raw.get(name)
        if value is not None:
            data[name] = value
    return data


def _content_part(part: Json) -> object:
    if isinstance(part, dict) and "text" in part:
        return SimpleNamespace(text=cast(dict[str, Json], part)["text"])
    return SimpleNamespace()


def _output_item(item: Json) -> object:
    fields = _object(item, "output item") if isinstance(item, dict) else None
    if fields is None:
        return item
    if "content" in fields:
        return SimpleNamespace(
            content=[_content_part(part) for part in _list(fields["content"], "content")]
        )
    if "text" in fields:
        return SimpleNamespace(text=fields["text"])
    return SimpleNamespace()


def _choice(choice: Json) -> object:
    fields = _object(choice, "choice")
    if "message" in fields:
        return types_utils.Choices(
            message=types_utils.Message(content=cast("str | None", fields["message"]))
        )
    return types_utils.StreamingChoices(
        delta=types_utils.Delta(content=cast("str | None", fields.get("delta")))
    )


def _response_object(response: dict[str, Json]) -> object:
    choices = response.get("choices")
    attributes: dict[str, object] = {
        "choices": [
            _choice(choice) for choice in ([] if choices is None else _list(choices, "choices"))
        ]
    }
    event = response.get("event")
    if event is not None:
        fields = _object(event, "event")
        attributes["type"] = _string(fields.get("event_type"), "event_type")
        output = fields.get("output")
        if output is not None:
            attributes["response"] = SimpleNamespace(
                output=[_output_item(item) for item in _list(output, "output")]
            )
        delta = fields.get("delta")
        if delta is not None:
            attributes["delta"] = delta
    return SimpleNamespace(**attributes)


def _execute(function: str, call: dict[str, Json]) -> Json:
    match function:
        case "count_characters":
            text = _string(call.get("text"), "text")
            return cast(Json, litellm_utils._count_characters(text))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        case "formatted_prompt":
            call_type = _prompt_call_type(_string(call.get("call_type"), "call_type"))
            data = _prompt_data(_object(call.get("data"), "data"))
            prompt = litellm_utils.get_formatted_prompt(  # pyright: ignore[reportUnknownMemberType, reportPrivateImportUsage]
                data=data, call_type=call_type
            )
            return cast(Json, prompt)
        case "response_string":
            response = _response_object(_object(call.get("response"), "response"))
            typed = cast("ModelResponse | ModelResponseStream", response)
            return cast(Json, litellm_utils.get_response_string(response_obj=typed))
        case _:
            raise ValueError(f"unknown function {function}")


def _run_call(call: dict[str, Json]) -> Json:
    try:
        value = _execute(_string(call.get("function"), "function"), call)
    except Exception as error:  # noqa: BLE001 - a rejected probe is part of the compared outcome
        outcome: dict[str, Json] = {"ok": False, "error": {"class": type(error).__name__}}
        return outcome
    return {"ok": True, "value": value}


def run(input_value: dict[str, Json]) -> Json:
    calls = input_value.get("calls")
    if not isinstance(calls, list):
        raise ValueError("calls must be a list")
    return [_run_call(_object(call, "call")) for call in cast(list[Json], calls)]
