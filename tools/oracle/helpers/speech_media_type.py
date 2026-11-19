from typing import TypedDict

from litellm.litellm_core_utils.audio_utils.utils import (
    resolve_speech_media_type,
    speech_media_type_from_audio_bytes,
)


class ResolveCase(TypedDict):
    upstream: str | None
    response_format: str | None


class SniffCase(TypedDict):
    audio: list[int]


class Input(TypedDict):
    resolve: list[ResolveCase]
    sniff: list[SniffCase]


class Output(TypedDict):
    resolve: list[str]
    sniff: list[str | None]


def run(input_value: Input) -> Output:
    resolved = [
        resolve_speech_media_type(case["upstream"], case["response_format"])
        for case in input_value["resolve"]
    ]
    sniffed = [
        speech_media_type_from_audio_bytes(bytes(case["audio"]))
        for case in input_value["sniff"]
    ]
    return {"resolve": resolved, "sniff": sniffed}
