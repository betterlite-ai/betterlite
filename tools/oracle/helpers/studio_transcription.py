import os
from collections.abc import Sequence
from typing import Final, Literal, Never, Protocol, TypeAlias, TypedDict

import httpx
from litellm.llms.gemini.audio_transcription.transformation import GeminiAudioTranscriptionConfig
from litellm.llms.gemini.common_utils import GeminiError
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class RequestData(Protocol):
    data: dict[str, JsonValue] | bytes


class ReplyData(Protocol):
    def model_dump_json(self, *, exclude_none: bool = False) -> str: ...


class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    @property
    def supports_subtitle_synthesis(self) -> bool: ...

    def map_openai_params(self, non_default_params: dict[str, JsonValue], optional_params: dict[str, JsonValue],
                          model: str, *, drop_params: bool) -> dict[str, JsonValue]: ...

    def validate_environment(self, headers: dict[str, str], model: str, messages: list[Never],
                             optional_params: dict[str, JsonValue],
                             litellm_params: dict[str, JsonValue]) -> dict[str, JsonValue]: ...

    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
            self, model: str, audio_file: bytes, optional_params: dict[str, JsonValue],
            litellm_params: dict[str, JsonValue]) -> RequestData: ...

    # ast-grep-ignore: no-long-names
    def transform_audio_transcription_response(self, raw_response: httpx.Response) -> ReplyData: ...


CONFIG: Config = GeminiAudioTranscriptionConfig()


class Supported(TypedDict):
    step: Literal["supported"]


class Subtitle(TypedDict):
    step: Literal["subtitle"]


class Params(TypedDict):
    step: Literal["params"]
    optional: dict[str, JsonValue]
    non_default: dict[str, JsonValue]


class Environment(TypedDict):
    step: Literal["environment"]
    headers: list[list[str]]
    api_key: str | None


class Request(TypedDict):
    step: Literal["request"]
    model: str
    content: str
    optional: dict[str, JsonValue]


class Reply(TypedDict):
    step: Literal["reply"]
    status: int
    headers: list[list[str]]
    body: str


Step: TypeAlias = Supported | Subtitle | Params | Environment | Request | Reply


class Input(TypedDict):
    steps: list[Step]


def guarded(step: Step) -> JsonValue:
    match step["step"]:
        case "supported":
            return JSON.validate_python(list(CONFIG.get_supported_openai_params("")))
        case "subtitle":
            return CONFIG.supports_subtitle_synthesis
        case "params":
            return JSON.validate_python(CONFIG.map_openai_params(
                non_default_params=step["non_default"], optional_params=step["optional"], model="", drop_params=False))
        case "environment":
            if step["api_key"] is None:
                os.environ.pop("GOOGLE_API_KEY", None)
            else:
                os.environ["GOOGLE_API_KEY"] = step["api_key"]
            return JSON.validate_python(CONFIG.validate_environment(
                {name: value for name, value in step["headers"]}, "", [], {}, {}))
        case "request":
            request = CONFIG.transform_audio_transcription_request(
                model=step["model"], audio_file=step["content"].encode(), optional_params=step["optional"],
                litellm_params={})
            return JSON.validate_python(request.data)
        case "reply":
            given = [(name, value) for name, value in step["headers"]]
            response = httpx.Response(step["status"], content=step["body"].encode(), headers=given)
            result = CONFIG.transform_audio_transcription_response(raw_response=response)
            return JSON.validate_json(result.model_dump_json(exclude_none=True))


def answer_or_raised(step: Step) -> JsonValue:
    try:
        return guarded(step)
    except GeminiError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/common_utils.py#L352
        return {"raised": error.status_code}
    except ValueError:
        return {"raised": None}


def run(input_value: Input) -> list[JsonValue]:
    steps = input_value["steps"]
    return [answer_or_raised(step) for step in steps]
