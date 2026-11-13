import os
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.xai.audio_transcription.transformation import (
    XAIAudioTranscriptionConfig,
    XAIAudioTranscriptionError,
)
from pydantic import JsonValue, TypeAdapter

FormData: TypeAlias = dict[str, str | list[str]]
FileParts: TypeAlias = dict[str, tuple[str, bytes, str]]

CLEARED: Final = ("XAI_API_KEY", "XAI_API_BASE")
CONFIG: Final = XAIAudioTranscriptionConfig.__new__(XAIAudioTranscriptionConfig)
FORM: Final[TypeAdapter[FormData]] = TypeAdapter(FormData)
PARTS: Final[TypeAdapter[FileParts]] = TypeAdapter(FileParts)
MAPPED: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
HEADERS: Final[TypeAdapter[dict[str, str]]] = TypeAdapter(dict[str, str])


class FormView(Protocol):
    data: dict[str, str | list[str]] | bytes
    files: dict[str, JsonValue] | None


class AudioSpec(TypedDict):
    filename: str | None
    content: str


AudioValue: TypeAlias = bytes | tuple[str, bytes]


class ReplyWord(TypedDict, total=False):
    word: str
    start: float
    end: float
    speaker: int


class ReplyAnswer(TypedDict, total=False):
    text: str
    task: str
    language: str
    duration: float
    words: list[ReplyWord]


class StatusError(TypedDict):
    status: int


class FailedReply(TypedDict):
    error: StatusError


class FilePart(TypedDict):
    filename: str
    content_type: str
    content: list[int]


class RequestCall(TypedDict):
    api_base: str | None
    api_key: str | None
    audio: AudioSpec
    model: str
    extra_body: dict[str, JsonValue]
    options: dict[str, JsonValue]


class ReplyCall(TypedDict):
    status: int
    headers: list[list[str]]
    body: str


class Common(TypedDict):
    env: dict[str, str]
    xai_key: str | None


class RequestInput(Common):
    operation: Literal["request"]
    calls: list[RequestCall]


class ReplyInput(Common):
    operation: Literal["reply"]
    calls: list[ReplyCall]


class RequestAnswer(TypedDict):
    supported: list[str]
    mapped: dict[str, JsonValue]
    url: str
    authorization: str | None
    raised: str | None
    form: list[list[str | list[str]]]
    file: FilePart | None


class RequestOutput(TypedDict):
    calls: list[RequestAnswer]


class ReplyOutput(TypedDict):
    calls: list[ReplyAnswer | FailedReply]


Input: TypeAlias = RequestInput | ReplyInput
Output: TypeAlias = RequestOutput | ReplyOutput
REPLY: Final[TypeAdapter[ReplyAnswer]] = TypeAdapter(ReplyAnswer)


def form_view(request: FormView) -> tuple[FormData, FileParts]:
    data = FORM.validate_python(request.data)
    parts = PARTS.validate_python(request.files)
    return data, parts


def audio_value(spec: AudioSpec) -> AudioValue:
    content = spec["content"].encode()
    filename = spec["filename"]
    if filename is None:
        return content
    return (filename, content)


def request_call(call: RequestCall) -> RequestAnswer:
    supported: list[str] = [str(name) for name in CONFIG.get_supported_openai_params(call["model"])]
    answer: RequestAnswer = {
        "supported": supported,
        "mapped": MAPPED.validate_python(
            CONFIG.map_openai_params(
                {"language": "en", "prompt": "direct"},
                {"response_format": "json", "language": "fr"},
                model="",
                drop_params=False,
            )
        ),
        "url": CONFIG.get_complete_url(
            api_base=call["api_base"],
            api_key=call["api_key"],
            model=call["model"],
            optional_params={},
            litellm_params={},
        ),
        "authorization": None,
        "raised": None,
        "form": [],
        "file": None,
    }
    try:
        headers = CONFIG.validate_environment(
            headers={},
            model=call["model"],
            messages=(),
            optional_params={},
            litellm_params={},
            api_key=call["api_key"],
            api_base=call["api_base"],
        )
    except ValueError:
        answer["raised"] = "key"
        return answer
    answer["authorization"] = HEADERS.validate_python(headers).get("Authorization")
    data, parts = form_view(
        CONFIG.transform_audio_transcription_request(
            model=call["model"],
            audio_file=audio_value(call["audio"]),
            optional_params={"extra_body": call["extra_body"], **call["options"]},
            litellm_params={},
        )
    )
    answer["form"] = [[name, value] for name, value in data.items()]
    filename, content, content_type = parts["file"]
    answer["file"] = {
        "filename": filename,
        "content_type": content_type,
        "content": list(content),
    }
    return answer


def reply_view(call: ReplyCall) -> ReplyAnswer | FailedReply:
    headers = [(name, value) for name, value in call["headers"]]
    response = httpx.Response(
        call["status"], content=call["body"].encode(), headers=headers
    )
    try:
        result = CONFIG.transform_audio_transcription_response(raw_response=response)
    except XAIAudioTranscriptionError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
        return {"error": {"status": error.status_code}}
    return REPLY.validate_json(result.model_dump_json(exclude_none=True))


def run(input_value: Input) -> Output:
    for name in CLEARED:
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    litellm.xai_key = input_value["xai_key"]
    match input_value["operation"]:
        case "request":
            return {"calls": [request_call(call) for call in input_value["calls"]]}
        case "reply":
            return {"calls": [reply_view(call) for call in input_value["calls"]]}
