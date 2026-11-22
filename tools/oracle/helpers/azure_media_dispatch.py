import base64
import json
from typing import Final, Literal, TypedDict, TypeAlias

import litellm
from pydantic import JsonValue, TypeAdapter

import azure_images
from azure_transcription.exchange import exchange as transcription_exchange
from azure_transcription.probe import Probe as TranscriptionProbe
from openai_transcription.probe import Answer

litellm.suppress_debug_info = True


class GenerationCase(TypedDict):
    case: Literal["image_generation"]
    api_base: str
    api_version: str
    model: str
    prompt: str
    api_key: str
    options: dict[str, JsonValue]
    reply: JsonValue


class EditCase(TypedDict):
    case: Literal["image_edit"]
    api_base: str
    api_version: str
    model: str
    prompt: str
    api_key: str
    options: dict[str, JsonValue]


class TranscriptionCase(TypedDict):
    case: Literal["transcription"]
    api_base: str
    api_version: str
    model: str
    api_key: str
    reply: JsonValue


Call: TypeAlias = GenerationCase | EditCase | TranscriptionCase


class Part(TypedDict):
    name: str
    filename: str | None
    content_type: str | None
    body: str


GENERATION: Final[TypeAdapter[GenerationCase]] = TypeAdapter(GenerationCase)
EDIT: Final[TypeAdapter[EditCase]] = TypeAdapter(EditCase)
TRANSCRIPTION: Final[TypeAdapter[TranscriptionCase]] = TypeAdapter(TranscriptionCase)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
PARTS: Final[TypeAdapter[list[Part]]] = TypeAdapter(list[Part])
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)


def generation_answer(case: GenerationCase) -> JsonValue:
    step: azure_images.Step = {
        "api_base": case["api_base"],
        "api_version": case["api_version"],
        "model": case["model"],
        "api_key": case["api_key"],
        "prompt": case["prompt"],
        "optional": case["options"],
        "reply_body": json.dumps(case["reply"]),
    }
    view = OBJECT.validate_python(azure_images.generation_step(step))
    headers = OBJECT.validate_python(view["sent_headers"])
    reply = OBJECT.validate_python(view["reply"])
    return {
        "sent_url": view["sent_url"],
        "sent_body": view["sent_body"],
        "api_key": headers["api-key"],
        "authorization": headers["authorization"],
        "created": reply["created"],
        "b64_json": reply["b64_json"],
        "image_url": reply["image_url"],
        "images": reply["images"],
    }


def edit_answer(case: EditCase) -> JsonValue:
    url = OBJECT.validate_python(
        azure_images.url_step(
            {
                "api_base": case["api_base"],
                "api_version": case["api_version"],
                "model": case["model"],
            }
        )
    )
    form = OBJECT.validate_python(
        azure_images.form_step(
            {
                "data": {
                    "model": case["model"],
                    "prompt": case["prompt"],
                    **case["options"],
                },
                "request_url": TEXT.validate_python(url["url"]),
            }
        )
    )
    auth = OBJECT.validate_python(azure_images.auth_step({"api_key": case["api_key"]}))
    fields = OBJECT.validate_python(form["fields"])
    return {
        "url": url["url"],
        "fields": [[name, fields[name]] for name in sorted(fields)],
        "auth": auth["headers"],
    }


def part_order(part: Part) -> str:
    return part["name"]


def transcription_answer(case: TranscriptionCase) -> JsonValue:
    probe = TranscriptionProbe(
        model=case["model"],
        api_key=case["api_key"],
        api_base=case["api_base"],
        api_version=case["api_version"],
        reply=Answer(
            status=200,
            headers={"content-type": "application/json"},
            body=json.dumps(case["reply"]),
        ),
    )
    view = OBJECT.validate_python(transcription_exchange(probe))
    request = OBJECT.validate_python(view["request"])
    result = OBJECT.validate_python(view["result"])
    parts = sorted(PARTS.validate_python(request["parts"]), key=part_order)
    entries: list[JsonValue] = [
        {
            "name": part["name"],
            "filename": part["filename"],
            "content_type": part["content_type"],
            "body": base64.b64decode(part["body"]).decode(),
        }
        for part in parts
    ]
    return {
        "url": request["url"],
        "api_key": request["api_key"],
        "accept": request["accept"],
        "parts": entries,
        "text": result["text"],
        "model": result["model"],
        "provider": result["provider"],
        "duration_recorded": result["duration_recorded"],
    }


def run(input_value: Call) -> JsonValue:
    if input_value["case"] == "image_generation":
        return generation_answer(GENERATION.validate_python(input_value))
    if input_value["case"] == "image_edit":
        return edit_answer(EDIT.validate_python(input_value))
    return transcription_answer(TRANSCRIPTION.validate_python(input_value))
