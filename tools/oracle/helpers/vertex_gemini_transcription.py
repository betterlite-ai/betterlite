import json
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.vertex_ai.audio_transcription.gemini_transcribe_transformation import (
    VertexGeminiAudioTranscriptionConfig,
)
from litellm.llms.vertex_ai.common_utils import VertexAIError
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class RequestData(Protocol):
    data: dict[str, JsonValue] | bytes


class ReplyData(Protocol):
    def model_dump_json(self, *, exclude_none: bool = False) -> str: ...


class Config(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        audio_file: bytes,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> RequestData: ...

    def transform_audio_transcription_response(  # ast-grep-ignore: no-long-names
        self, raw_response: httpx.Response
    ) -> ReplyData: ...


CONFIG: Config = VertexGeminiAudioTranscriptionConfig()


class Url(TypedDict):
    step: Literal["url"]
    api_base: str | None
    location: str | None
    project: str | None
    model: str


class Request(TypedDict):
    step: Literal["request"]
    content: str
    language: JsonValue | None


class Reply(TypedDict):
    step: Literal["reply"]
    status: int
    headers: list[list[str]]
    body: str


Step: TypeAlias = Url | Request | Reply


class Input(TypedDict):
    steps: list[Step]


def url_step(step: Url) -> JsonValue:
    return CONFIG.get_complete_url(
        api_base=step["api_base"],
        api_key=None,
        model=step["model"],
        optional_params={},
        litellm_params={"vertex_project": step["project"], "vertex_location": step["location"]},
    )


def request_step(step: Request) -> JsonValue:
    optional_params: dict[str, JsonValue] = {}
    if step["language"] is not None:
        optional_params["language"] = step["language"]
    request = CONFIG.transform_audio_transcription_request(
        model="",
        audio_file=step["content"].encode(),
        optional_params=optional_params,
        litellm_params={},
    )
    return JSON.validate_json(json.dumps(request.data))


def reply_step(step: Reply) -> JsonValue:
    given = [(name, value) for name, value in step["headers"]]
    response = httpx.Response(step["status"], content=step["body"].encode(), headers=given)
    result = CONFIG.transform_audio_transcription_response(raw_response=response)
    return JSON.validate_json(result.model_dump_json(exclude_none=True))


def guarded(step: Step) -> JsonValue:
    try:
        match step["step"]:
            case "url":
                return url_step(step)
            case "request":
                return request_step(step)
            case "reply":
                return reply_step(step)
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        return {"raised": error.status_code}
    except ValueError:
        return {"raised": None}


def run(input_value: Input) -> list[JsonValue]:
    return [
        guarded(step) for step in input_value["steps"]
    ]
