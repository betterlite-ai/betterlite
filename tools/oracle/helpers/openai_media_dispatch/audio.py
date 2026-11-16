import base64

import litellm
from pydantic import JsonValue

from openai_speech import run as speech_outcome
from openai_speech.types import (
    ClientSpec,
)
from openai_speech.types import (
    Input as SpeechInput,
)
from openai_speech.types import (
    Reply as SpeechReply,
)
from openai_speech.types import (
    Request as SpeechRequest,
)
from openai_transcription.exchange import exchange as transcription_outcome
from openai_transcription.probe import Answer as TranscriptionAnswer
from openai_transcription.probe import Probe as TranscriptionProbe

from .cases import AudioCases, SpeechCase, TranscriptionCase
from .support import BYTES, OBJECT, OBJECTS, SPEECH_MAX_RETRIES, TEXT, TIMEOUT

litellm.suppress_debug_info = True


def speech(case: SpeechCase) -> JsonValue:
    options = case["options"]
    request: SpeechRequest = {
        "model": case["model"],
        "input": case["input"],
        "voice": case["voice"],
    }
    if "response_format" in options:
        request["response_format"] = options["response_format"]
    if "speed" in options:
        request["speed"] = options["speed"]
    if "instructions" in options:
        request["instructions"] = options["instructions"]
    if "extra_headers" in options:
        request["extra_headers"] = OBJECT.validate_python(options["extra_headers"])
    headers: list[list[str]] = []
    content_type = case["content_type"]
    if content_type is not None:
        headers.append(["content-type", content_type])
    client: ClientSpec = {
        "timeout": TIMEOUT,
        "max_retries": SPEECH_MAX_RETRIES,
        "api_key": "sk-speech",
        "api_base": case["api_base"],
        "aspeech": False,
    }
    reply: SpeechReply = {"status": case["status"], "headers": headers, "text": case["body"]}
    probe: SpeechInput = {"client": client, "request": request, "reply": reply}
    outcome = OBJECT.validate_python(speech_outcome(probe))
    sent = OBJECT.validate_python(outcome["request"])
    sent_view: dict[str, JsonValue] = {
        "method": sent["method"],
        "url": sent["url"],
        "accept": sent["accept"],
        "authorization": sent["authorization"],
        "extra_headers": sent["extra_headers"],
        "body": sent["body"],
    }
    result = OBJECT.validate_python(outcome["result"])
    raised = result.get("raised")
    if raised is not None:
        error = OBJECT.validate_python(raised)
        return {
            "request": sent_view,
            "status": None,
            "content_type": None,
            "body": None,
            "error": {
                "status": error["status"],
                "type": error["type"],
                "param": error["param"],
                "code": error["code"],
            },
        }
    content = BYTES.validate_python(result["body"])
    return {
        "request": sent_view,
        "status": result["status"],
        "content_type": result["content_type"],
        "body": content.decode(),
        "error": None,
    }


def transcription(case: TranscriptionCase) -> JsonValue:
    probe = TranscriptionProbe(
        model=case["model"],
        provider=case["family"],
        optional_params=case["options"],
        headers={},
        api_key="sk-transcription",
        api_base=case["api_base"],
        reply=TranscriptionAnswer(status=case["status"], body=case["reply"]),
    )
    outcome = OBJECT.validate_python(transcription_outcome(probe))
    sent = OBJECT.validate_python(outcome["request"])
    parts: list[JsonValue] = []
    for part in OBJECTS.validate_python(sent["parts"]):
        fields = OBJECT.validate_python(part)
        encoded = TEXT.validate_python(fields["body"])
        parts.append(
            {
                "name": fields["name"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": base64.b64decode(encoded).decode(),
            }
        )
    request: dict[str, JsonValue] = {
        "url": sent["url"],
        "authorization": sent["authorization"],
        "accept": sent["accept"],
        "parts": parts,
    }
    error = outcome["error"]
    if error is not None:
        raised = OBJECT.validate_python(error)
        return {
            "request": request,
            "reply": None,
            "error": {
                "status": raised["status"],
                "type": raised["type"],
                "param": raised["param"],
                "code": raised["code"],
            },
        }
    result = OBJECT.validate_python(outcome["result"])
    response = OBJECT.validate_python(result["response"])
    return {
        "request": request,
        "reply": {
            "text": response["text"],
            "fields": response["fields"],
            "model": result["model"],
            "provider": result["provider"],
            "duration_recorded": result["duration_recorded"],
        },
        "error": None,
    }


def audio_cases(case: AudioCases) -> JsonValue:
    match case["case"]:
        case "speech":
            return speech(case)
        case "transcription":
            return transcription(case)
