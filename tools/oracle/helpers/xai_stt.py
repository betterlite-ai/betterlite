import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.xai.audio_transcription.transformation import XAIAudioTranscriptionConfig
from pydantic import JsonValue, TypeAdapter

from xai_files import guarded

CONFIG: Final = XAIAudioTranscriptionConfig.__new__(XAIAudioTranscriptionConfig)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
FORM: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
PARTS: Final[TypeAdapter[dict[str, tuple[str, bytes, str]]]] = TypeAdapter(
    dict[str, tuple[str, bytes, str]]
)


class FormView(Protocol):
    data: dict[str, str | list[str]] | bytes
    files: dict[str, JsonValue] | None


class Params(TypedDict):
    step: Literal["params"]
    optional: dict[str, JsonValue]
    non_default: dict[str, JsonValue]


class Url(TypedDict):
    step: Literal["url"]
    api_base: str | None


class Auth(TypedDict):
    step: Literal["auth"]
    api_key: str | None


class Form(TypedDict):
    step: Literal["form"]
    model: str
    filename: str | None
    content: str
    extra_body: dict[str, JsonValue]
    options: dict[str, JsonValue]


class Reply(TypedDict):
    step: Literal["reply"]
    status: int
    headers: list[list[str]]
    body: str


Step: TypeAlias = Params | Url | Auth | Form | Reply


class Input(TypedDict):
    env: dict[str, str]
    xai_key: str | None
    steps: list[Step]


def form_answer(request: FormView) -> JsonValue:
    form: list[JsonValue] = [
        [name, value] for name, value in FORM.validate_python(request.data).items()
    ]
    filename, body, content_type = PARTS.validate_python(request.files)["file"]
    return {"form": form, "file": [filename, content_type, list(body)]}


def form_step(step: Form) -> JsonValue:
    content = step["content"].encode()
    audio = content if step["filename"] is None else (step["filename"], content)
    return form_answer(
        CONFIG.transform_audio_transcription_request(
            model=step["model"],
            audio_file=audio,
            optional_params={"extra_body": step["extra_body"], **step["options"]},
            litellm_params={},
        )
    )


def outcome(step: Step) -> JsonValue:
    match step["step"]:
        case "params":
            mapped = CONFIG.map_openai_params(
                step["non_default"], step["optional"], model="", drop_params=False
            )
            supported: list[JsonValue] = [
                str(name) for name in CONFIG.get_supported_openai_params("")
            ]
            return {"supported": supported, "mapped": JSON.validate_python(mapped)}
        case "url":
            return CONFIG.get_complete_url(
                api_base=step["api_base"],
                api_key=None,
                model="",
                optional_params={},
                litellm_params={},
            )
        case "auth":
            headers = CONFIG.validate_environment(
                headers={},
                model="",
                messages=(),
                optional_params={},
                litellm_params={},
                api_key=step["api_key"],
            )
            return JSON.validate_python(headers.get("Authorization"))
        case "form":
            return form_step(step)
        case "reply":
            given = [(name, value) for name, value in step["headers"]]
            response = httpx.Response(step["status"], content=step["body"].encode(), headers=given)
            result = CONFIG.transform_audio_transcription_response(raw_response=response)
            return JSON.validate_json(result.model_dump_json(exclude_none=True))


def run(input_value: Input) -> list[JsonValue]:
    for name in ("XAI_API_KEY", "XAI_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    litellm.xai_key = input_value["xai_key"]
    return [guarded(outcome, step) for step in input_value["steps"]]
