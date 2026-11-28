from pydantic import JsonValue

import bedrock_other_transcription

from .common import (
    JSON,
    UNSUPPORTED_FORMAT,
    failure_row,
    params_of,
    success_row,
)
from .models import (
    TRANSCRIPTION_INPUT,
    TRANSCRIPTION_OUTPUT,
    File,
    Probe,
)


def file_case(file_value: File) -> JsonValue:
    content = file_value["content"]
    filename = file_value.get("filename")
    raw: dict[str, JsonValue] = {"kind": "bytes", "content": content}
    if filename is not None:
        raw = {"kind": "tuple", "filename": filename, "content": content}
    return raw


def transcription_call(probe: Probe) -> dict[str, JsonValue]:
    file_value = probe.get("file")
    if file_value is None:
        raise ValueError("a transcription probe carries a file")
    mode = "async" if probe.get("asynchronous") else "sync"
    raw: dict[str, JsonValue] = {
        "mode": mode,
        "model": probe["model"],
        "file": file_case(file_value),
        "custom_llm_provider": "bedrock",
        "optional_params": params_of(probe),
        "api_key": probe.get("api_key"),
        "api_base": probe.get("api_base"),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "timeout": probe.get("timeout"),
        "reply_text": probe.get("reply_text"),
    }
    return raw


def transcription_row(probe: Probe) -> JsonValue:
    calls: list[JsonValue] = [transcription_call(probe)]
    raw: dict[str, JsonValue] = {"calls": calls}
    output = TRANSCRIPTION_OUTPUT.validate_python(
        bedrock_other_transcription.run(TRANSCRIPTION_INPUT.validate_python(raw))
    )
    call = output["calls"][0]
    if not call["ok"]:
        return failure_row([], None, UNSUPPORTED_FORMAT)
    text = call.get("text")
    if text is None:
        text = ""
    value: dict[str, JsonValue] = {"text": text, "seen": call["seen"]}
    return success_row([], value)
