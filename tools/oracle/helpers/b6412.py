import os
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.ovhcloud.audio_transcription.transformation import (
    OVHCloudAudioTranscriptionConfig,
)
from litellm.llms.ovhcloud.chat.transformation import (
    OVHCloudChatCompletionStreamingHandler,
    OVHCloudChatConfig,
)
from litellm.llms.ovhcloud.embedding.transformation import OVHCloudEmbeddingConfig
from litellm.llms.scaleway.audio_transcription.transformation import (
    ScalewayAudioTranscriptionConfig,
)
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True
EMPTY: dict[str, JsonValue] = {}
NO_HEADERS: dict[str, str] = {}
NO_MESSAGES: list[dict[str, JsonValue]] = []


class ChunkView(Protocol):
    @property
    def id(self) -> str: ...

    @property
    def object(self) -> str: ...

    @property
    def created(self) -> int: ...

    @property
    def model(self) -> str | None: ...


class ReplyView(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...


class RequestData(Protocol):
    data: dict[str, JsonValue]
    files: dict[str, tuple[str, bytes, str]]


class Config(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        stream: bool | None = None,
    ) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...


class ChatConfig(Config, Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...


class EmbeddingConfig(Config, Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_embedding_request(
        self,
        model: str,
        input_value: JsonValue,
        optional_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...

    def transform_embedding_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: object,
        logging_obj: object,
        api_key: str | None,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> ReplyView: ...


class AudioConfig(Config, Protocol):
    def transform_audio_transcription_request(
        self,
        model: str,
        audio_file: tuple[str, bytes],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> RequestData: ...

    def transform_audio_transcription_response(
        self,
        raw_response: httpx.Response,
    ) -> ReplyView: ...


class Handler(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ChunkView: ...


class ChatRequestCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: dict[str, JsonValue]


class ChatRequestInput(TypedDict):
    function: Literal["chat_request"]
    requests: list[ChatRequestCall]


class ChunksInput(TypedDict):
    function: Literal["chunks"]
    chunks: list[dict[str, JsonValue]]


class HeaderCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]
    env: dict[str, str]


class EmbeddingCall(TypedDict):
    model: str
    input: JsonValue
    non_default_params: dict[str, JsonValue]


class ReplyCall(TypedDict):
    body: str
    status: int
    headers: dict[str, str]


class EmbeddingInput(TypedDict):
    function: Literal["embedding"]
    headers: list[HeaderCall]
    requests: list[EmbeddingCall]
    replies: list[ReplyCall]


class TranscriptionCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: dict[str, JsonValue]
    filename: str
    content: str
    env: dict[str, str]


class TranscriptionInput(TypedDict):
    function: Literal["transcription"]
    provider: Literal["ovhcloud", "scaleway"]
    requests: list[TranscriptionCall]
    replies: list[ReplyCall]


class ScalewayKeyInput(TypedDict):
    function: Literal["scaleway_key"]
    calls: list[HeaderCall]
    replies: list[ReplyCall]


Input: TypeAlias = (
    ChatRequestInput
    | ChunksInput
    | EmbeddingInput
    | TranscriptionInput
    | ScalewayKeyInput
)

OVH_EMBEDDING: EmbeddingConfig = OVHCloudEmbeddingConfig()
OVH_AUDIO: AudioConfig = OVHCloudAudioTranscriptionConfig()
SCALEWAY_AUDIO: AudioConfig = ScalewayAudioTranscriptionConfig()


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value


def response_of(call: ReplyCall) -> httpx.Response:
    return httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=call["headers"],
        request=httpx.Request("POST", "https://host"),
    )


def chat_request(config: ChatConfig, call: ChatRequestCall) -> JsonValue:
    url = config.get_complete_url(
        api_base=call["api_base"],
        api_key=None,
        model=call["model"],
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    try:
        body = config.transform_request(
            model=call["model"],
            messages=NO_MESSAGES,
            optional_params=optional,
            litellm_params=EMPTY,
            headers=EMPTY,
        )
    except (TypeError, ValueError):
        return {"url": url, "ok": False}
    return {"url": url, "ok": True, "value": body}


def chunk_value(handler: Handler, chunk: dict[str, JsonValue]) -> JsonValue:
    try:
        parsed = handler.chunk_parser(chunk)
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    return {
        "ok": True,
        "value": {
            "id": parsed.id,
            "object": parsed.object,
            "created": parsed.created,
            "model": parsed.model,
            "usage": chunk.get("usage"),
            "choices": chunk["choices"],
        },
    }


def header_value(config: Config, call: HeaderCall) -> JsonValue:
    apply_env(call["env"])
    try:
        headers = config.validate_environment(
            headers=dict(call["headers"]),
            model="m",
            messages=NO_MESSAGES,
            optional_params=EMPTY,
            litellm_params=EMPTY,
            api_key=call["api_key"],
            api_base=None,
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "headers": headers}


def embedding_request(config: EmbeddingConfig, call: EmbeddingCall) -> JsonValue:
    mapped = config.map_openai_params(
        non_default_params=dict(call["non_default_params"]),
        optional_params={},
        model=call["model"],
        drop_params=False,
    )
    body = config.transform_embedding_request(
        call["model"], call["input"], mapped, EMPTY
    )
    return {"value": [[name, value] for name, value in body.items()]}


def embedding_reply(config: EmbeddingConfig, call: ReplyCall) -> JsonValue:
    try:
        result = config.transform_embedding_response(
            model="m",
            raw_response=response_of(call),
            model_response=EmbeddingResponse(),
            logging_obj=EMPTY,
            api_key=None,
            request_data=EMPTY,
            optional_params=EMPTY,
            litellm_params=EMPTY,
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    dumped = result.model_dump()
    usage = TypeAdapter(dict[str, JsonValue]).validate_python(dumped["usage"])
    return {
        "ok": True,
        "value": {
            "model": dumped["model"],
            "data": dumped["data"],
            "object": dumped["object"],
            "usage": {
                "prompt_tokens": TypeAdapter(int).validate_python(
                    usage["prompt_tokens"]
                ),
                "total_tokens": TypeAdapter(int).validate_python(usage["total_tokens"]),
            },
        },
    }


def transcription_request(config: AudioConfig, call: TranscriptionCall) -> JsonValue:
    apply_env(call["env"])
    url = config.get_complete_url(
        api_base=call["api_base"],
        api_key=None,
        model=call["model"],
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    try:
        headers = config.validate_environment(
            headers=NO_HEADERS,
            model=call["model"],
            messages=NO_MESSAGES,
            optional_params=EMPTY,
            litellm_params=EMPTY,
            api_key=None,
            api_base=None,
        )
    except BaseLLMException as error:
        return {"url": url, "ok": False, "status": error.status_code}
    data = config.transform_audio_transcription_request(
        model=call["model"],
        audio_file=(call["filename"], call["content"].encode()),
        optional_params=dict(call["optional_params"]),
        litellm_params=EMPTY,
    )
    filename, content, content_type = data.files["file"]
    return {
        "url": url,
        "ok": True,
        "headers": headers,
        "form": [[name, value] for name, value in data.data.items()],
        "file": {
            "filename": filename,
            "content": content.decode(),
            "content_type": content_type,
        },
    }


def transcription_reply(config: AudioConfig, call: ReplyCall) -> JsonValue:
    try:
        result = config.transform_audio_transcription_response(
            raw_response=response_of(call)
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    dumped = result.model_dump()
    value: dict[str, JsonValue] = {"text": dumped.get("text")}
    for name in ("segments", "language"):
        if name in dumped:
            value[name] = dumped[name]
    return {"ok": True, "value": value}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "chunks":
        handler: Handler = OVHCloudChatCompletionStreamingHandler(
            streaming_response=iter([""]), sync_stream=False, json_mode=False
        )
        return [chunk_value(handler, chunk) for chunk in input_value["chunks"]]
    if input_value["function"] == "chat_request":
        chat: ChatConfig = OVHCloudChatConfig()
        return [chat_request(chat, call) for call in input_value["requests"]]
    if input_value["function"] == "embedding":
        return {
            "headers": [
                header_value(OVH_EMBEDDING, call) for call in input_value["headers"]
            ],
            "requests": [
                embedding_request(OVH_EMBEDDING, call)
                for call in input_value["requests"]
            ],
            "replies": [
                embedding_reply(OVH_EMBEDDING, call) for call in input_value["replies"]
            ],
        }
    if input_value["function"] == "scaleway_key":
        return {
            "headers": [
                header_value(SCALEWAY_AUDIO, call) for call in input_value["calls"]
            ],
            "replies": [
                transcription_reply(SCALEWAY_AUDIO, reply)
                for reply in input_value["replies"]
            ],
        }
    audio = OVH_AUDIO if input_value["provider"] == "ovhcloud" else SCALEWAY_AUDIO
    return {
        "requests": [
            transcription_request(audio, call) for call in input_value["requests"]
        ],
        "replies": [
            transcription_reply(audio, reply) for reply in input_value["replies"]
        ],
    }
