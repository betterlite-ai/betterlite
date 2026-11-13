import os
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.ovhcloud.audio_transcription.transformation import (
    OVHCloudAudioTranscriptionConfig,
)
from litellm.llms.ovhcloud.chat.transformation import (
    OVHCloudChatCompletionStreamingHandler as OvhHandler,
)
from litellm.llms.ovhcloud.chat.transformation import OVHCloudChatConfig
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
    id: str
    object: str
    created: int
    model: str | None


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
    ) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
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
        *,
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
        *rest: JsonValue,
    ) -> ReplyView: ...


class AudioConfig(Config, Protocol):
    def transform_audio_transcription_request(
        self,
        model: str,
        audio_file: tuple[str, bytes],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> RequestData: ...

    def transform_audio_transcription_response(self, raw_response: httpx.Response) -> ReplyView: ...


class Handler(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ChunkView: ...


class ChatRequestCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: dict[str, JsonValue]


class HeaderCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]
    env: dict[str, str]


class ReplyCall(TypedDict):
    body: str
    status: int
    headers: dict[str, str]


class EmbeddingCall(TypedDict):
    model: str
    input: JsonValue
    non_default_params: dict[str, JsonValue]


class TranscriptionCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: dict[str, JsonValue]
    filename: str
    content: str
    env: dict[str, str]


class ChatRequestInput(TypedDict):
    function: Literal["chat_request"]
    requests: list[ChatRequestCall]


class ChunksInput(TypedDict):
    function: Literal["chunks"]
    chunks: list[dict[str, JsonValue]]


class EmbeddingInput(TypedDict):
    function: Literal["embedding"]
    headers: list[HeaderCall]
    requests: list[EmbeddingCall]
    replies: list[ReplyCall]


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
    ChatRequestInput | ChunksInput | EmbeddingInput | TranscriptionInput | ScalewayKeyInput
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
    url = config.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY)
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    try:
        body = config.transform_request(call["model"], NO_MESSAGES, optional, EMPTY, EMPTY)
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
            dict(call["headers"]), "m", NO_MESSAGES, EMPTY, dict(EMPTY)
        )
    except BaseLLMException as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "headers": headers}


def embedding_request(config: EmbeddingConfig, call: EmbeddingCall) -> JsonValue:
    mapped = config.map_openai_params(
        dict(call["non_default_params"]), {}, call["model"], drop_params=False
    )
    body = config.transform_embedding_request(call["model"], call["input"], mapped, EMPTY)
    return {"value": [[name, value] for name, value in body.items()]}


def embedding_reply(config: EmbeddingConfig, call: ReplyCall) -> JsonValue:
    try:
        result = config.transform_embedding_response(
            "m", response_of(call), EmbeddingResponse(), EMPTY, None, EMPTY, EMPTY, EMPTY
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
                "prompt_tokens": TypeAdapter(int).validate_python(usage["prompt_tokens"]),
                "total_tokens": TypeAdapter(int).validate_python(usage["total_tokens"]),
            },
        },
    }


def transcription_request(config: AudioConfig, call: TranscriptionCall) -> JsonValue:
    apply_env(call["env"])
    url = config.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY)
    try:
        headers = config.validate_environment(
            NO_HEADERS, call["model"], NO_MESSAGES, EMPTY, dict(EMPTY)
        )
    except BaseLLMException as error:
        return {"url": url, "ok": False, "status": error.status_code}
    audio = (call["filename"], call["content"].encode())
    data = config.transform_audio_transcription_request(
        call["model"], audio, dict(call["optional_params"]), EMPTY
    )
    filename, content, content_type = data.files["file"]
    return {
        "url": url,
        "ok": True,
        "headers": headers,
        "form": [[name, value] for name, value in data.data.items()],
        "file": {"filename": filename, "content": content.decode(), "content_type": content_type},
    }


def transcription_reply(config: AudioConfig, call: ReplyCall) -> JsonValue:
    try:
        result = config.transform_audio_transcription_response(response_of(call))
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
        handler: Handler = OvhHandler(
            streaming_response=iter([""]), sync_stream=False, json_mode=False
        )
        return [chunk_value(handler, chunk) for chunk in input_value["chunks"]]
    if input_value["function"] == "chat_request":
        chat: ChatConfig = OVHCloudChatConfig()
        return [chat_request(chat, call) for call in input_value["requests"]]
    if input_value["function"] == "embedding":
        return {
            "headers": [header_value(OVH_EMBEDDING, call) for call in input_value["headers"]],
            "requests": [
                embedding_request(OVH_EMBEDDING, call) for call in input_value["requests"]
            ],
            "replies": [embedding_reply(OVH_EMBEDDING, call) for call in input_value["replies"]],
        }
    if input_value["function"] == "scaleway_key":
        return {
            "headers": [header_value(SCALEWAY_AUDIO, call) for call in input_value["calls"]],
            "replies": [
                transcription_reply(SCALEWAY_AUDIO, reply) for reply in input_value["replies"]
            ],
        }
    audio = OVH_AUDIO if input_value["provider"] == "ovhcloud" else SCALEWAY_AUDIO
    return {
        "requests": [transcription_request(audio, call) for call in input_value["requests"]],
        "replies": [transcription_reply(audio, reply) for reply in input_value["replies"]],
    }
