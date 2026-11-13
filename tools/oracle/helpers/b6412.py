import os
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.audio_transcription.transformation import BaseAudioTranscriptionConfig
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
from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

from xai_files import LOG, guarded
from xai_stt import form_answer

Object: TypeAlias = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[Object]] = TypeAdapter(Object)
EMPTY: Final[Object] = {}
litellm.suppress_debug_info = True


class NoMessages(list[AllMessageValues]):
    pass


class Keyed(Protocol):
    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: Object,
        litellm_params: Object,
        /,
        *,
        api_key: str | None = None,
    ) -> dict[str, str]: ...


class ChatConfig(Protocol):
    def get_complete_url(
        self, base: str | None, key: str | None, model: str, params: Object, extra: Object, /
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: Object,
        litellm_params: Object,
        headers: Object,
        /,
    ) -> Object: ...


class EmbeddingConfig(Keyed, Protocol):
    def map_openai_params(
        self, non_default: Object, optional: Object, model: str, /, *, drop_params: bool
    ) -> Object: ...

    def transform_embedding_request(
        self, model: str, value: AllEmbeddingInputValues, optional: Object, headers: Object, /
    ) -> Object: ...

    def transform_embedding_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: EmbeddingResponse,
        logging_obj: Logging,
        api_key: str | None,
        request_data: Object,
        optional_params: Object,
        litellm_params: Object,
        /,
    ) -> EmbeddingResponse: ...


class UsageView(TypedDict):
    prompt_tokens: int
    total_tokens: int


class ChatRequestCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: Object


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
    input: AllEmbeddingInputValues
    non_default_params: Object


class TranscriptionCall(TypedDict):
    api_base: str | None
    model: str
    optional_params: Object
    filename: str
    content: str
    env: dict[str, str]


class ChatRequestInput(TypedDict):
    function: Literal["chat_request"]
    requests: list[ChatRequestCall]


class ChunksInput(TypedDict):
    function: Literal["chunks"]
    chunks: list[Object]


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

OVH_EMBEDDING: Final[EmbeddingConfig] = OVHCloudEmbeddingConfig()
OVH_AUDIO: Final = OVHCloudAudioTranscriptionConfig.__new__(OVHCloudAudioTranscriptionConfig)
SCALEWAY_AUDIO: Final = ScalewayAudioTranscriptionConfig.__new__(ScalewayAudioTranscriptionConfig)


def response_of(call: ReplyCall) -> httpx.Response:
    return httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=call["headers"],
        request=httpx.Request("POST", "https://host"),
    )


def chat_request(config: ChatConfig, call: ChatRequestCall) -> JsonValue:
    url = config.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY)
    optional = dict(call["optional_params"])
    body = config.transform_request(call["model"], NoMessages(), optional, EMPTY, EMPTY)
    return {"url": url, "ok": True, "value": body}


def chunk_answer(handler: OvhHandler, chunk: Object) -> JsonValue:
    parsed = handler.chunk_parser(chunk)
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


def auth_headers(config: Keyed, call: HeaderCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    headers = config.validate_environment(
        dict(call["headers"]), "m", NoMessages(), EMPTY, EMPTY, api_key=call["api_key"]
    )
    return {"ok": True, "headers": JSON.validate_python(headers)}


def embedding_request(config: EmbeddingConfig, call: EmbeddingCall) -> JsonValue:
    mapped = config.map_openai_params(
        call["non_default_params"], {}, call["model"], drop_params=False
    )
    body = config.transform_embedding_request(call["model"], call["input"], mapped, EMPTY)
    return {"value": [[name, value] for name, value in body.items()]}


def embedding_reply(config: EmbeddingConfig, call: ReplyCall) -> JsonValue:
    result = config.transform_embedding_response(
        "m", response_of(call), EmbeddingResponse(), LOG, None, EMPTY, EMPTY, EMPTY
    )
    dumped = OBJECT.validate_json(result.model_dump_json())
    usage = TypeAdapter(UsageView).validate_python(dumped["usage"])
    return {
        "ok": True,
        "value": {
            "model": dumped["model"],
            "data": dumped["data"],
            "object": dumped["object"],
            "usage": {
                "prompt_tokens": usage["prompt_tokens"],
                "total_tokens": usage["total_tokens"],
            },
        },
    }


def transcription_request(
    config: BaseAudioTranscriptionConfig, call: TranscriptionCall
) -> JsonValue:
    url = config.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY)
    keyed = HeaderCall(api_key=None, headers={}, env=call["env"])
    audio = (call["filename"], call["content"].encode())
    request = config.transform_audio_transcription_request(
        call["model"], audio, dict(call["optional_params"]), EMPTY
    )
    return {
        "url": url,
        "headers": guarded(partial(auth_headers, config), keyed),
        "request": form_answer(request),
    }


def transcription_reply(config: BaseAudioTranscriptionConfig, call: ReplyCall) -> JsonValue:
    result = config.transform_audio_transcription_response(response_of(call))
    dumped = OBJECT.validate_json(result.model_dump_json())
    value: Object = {"text": dumped.get("text")}
    for name in ("segments", "language"):
        if name in dumped:
            value[name] = dumped[name]
    return {"ok": True, "value": value}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "chunks":
        handler = OvhHandler(streaming_response=iter([""]), sync_stream=False, json_mode=False)
        return [guarded(partial(chunk_answer, handler), item) for item in input_value["chunks"]]
    if input_value["function"] == "chat_request":
        chat: ChatConfig = OVHCloudChatConfig()
        return [guarded(partial(chat_request, chat), item) for item in input_value["requests"]]
    if input_value["function"] == "embedding":
        headers = partial(auth_headers, OVH_EMBEDDING)
        requests = partial(embedding_request, OVH_EMBEDDING)
        replies = partial(embedding_reply, OVH_EMBEDDING)
        return {
            "headers": [guarded(headers, item) for item in input_value["headers"]],
            "requests": [guarded(requests, item) for item in input_value["requests"]],
            "replies": [guarded(replies, item) for item in input_value["replies"]],
        }
    if input_value["function"] == "scaleway_key":
        keys = partial(auth_headers, SCALEWAY_AUDIO)
        replies = partial(transcription_reply, SCALEWAY_AUDIO)
        return {
            "headers": [guarded(keys, item) for item in input_value["calls"]],
            "replies": [guarded(replies, item) for item in input_value["replies"]],
        }
    audio: BaseAudioTranscriptionConfig = OVH_AUDIO
    if input_value["provider"] == "scaleway":
        audio = SCALEWAY_AUDIO
    forms = partial(transcription_request, audio)
    transcripts = partial(transcription_reply, audio)
    return {
        "requests": [guarded(forms, item) for item in input_value["requests"]],
        "replies": [guarded(transcripts, item) for item in input_value["replies"]],
    }
