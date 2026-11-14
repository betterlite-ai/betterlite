from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.litellm_core_utils.audio_utils.subtitle_utils import (
    SubtitleToken,
    group_subtitle_tokens_into_cues,
    render_subtitle_tokens_as_srt,
    render_subtitle_tokens_as_vtt,
    synthesize_subtitle_document,
)
from pydantic import JsonValue


class Token(TypedDict):
    text: str
    start_ms: NotRequired[int | None]
    end_ms: NotRequired[int | None]
    speaker: NotRequired[str | int | None]


class GroupCall(TypedDict):
    function: Literal["group"]
    tokens: list[Token]


class SrtCall(TypedDict):
    function: Literal["srt"]
    tokens: list[Token]


class VttCall(TypedDict):
    function: Literal["vtt"]
    tokens: list[Token]


class SynthesizeCall(TypedDict):
    function: Literal["synthesize"]
    words: JsonValue
    response_format: str


Call: TypeAlias = GroupCall | SrtCall | VttCall | SynthesizeCall


class Input(TypedDict):
    calls: list[Call]


def run(input_value: Input) -> JsonValue:
    return [
        _run_one(call)
        for call in input_value["calls"]
    ]


def _run_one(call: Call) -> JsonValue:
    outcome: JsonValue
    try:
        outcome = {"ok": True, "value": _evaluate(call)}
    except (ValueError, OverflowError):
        outcome = {"ok": False}
    return outcome


def _evaluate(call: Call) -> JsonValue:
    match call["function"]:
        case "group":
            return _cues(_tokens(call["tokens"]))
        case "srt":
            return render_subtitle_tokens_as_srt(_tokens(call["tokens"]))
        case "vtt":
            return render_subtitle_tokens_as_vtt(_tokens(call["tokens"]))
        case "synthesize":
            return synthesize_subtitle_document(call["words"], call["response_format"])


def _tokens(values: list[Token]) -> list[SubtitleToken]:
    return [
        SubtitleToken(
            text=value["text"],
            start_ms=value.get("start_ms"),
            end_ms=value.get("end_ms"),
            speaker=value.get("speaker"),
        )
        for value in values
    ]


def _cues(tokens: list[SubtitleToken]) -> list[JsonValue]:
    return [
        {"start_ms": cue.start_ms, "end_ms": cue.end_ms, "text": cue.text}
        for cue in group_subtitle_tokens_into_cues(tokens)
    ]
