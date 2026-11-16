import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.audio_transcription.transformation import (
    AudioTranscriptionRequestData,
)
from litellm.llms.watsonx.audio_transcription.transformation import (
    IBMWatsonXAudioTranscriptionConfig as WatsonxAudio,
)
from litellm.llms.watsonx.embed.transformation import IBMWatsonXEmbeddingConfig
from litellm.llms.watsonx.passthrough.transformation import WatsonxPassthroughConfig
from litellm.llms.watsonx.rerank.transformation import IBMWatsonXRerankConfig
from litellm.types.llms.openai import AllEmbeddingInputValues
from litellm.types.rerank import RerankResponse
from litellm.types.utils import EmbeddingResponse, TranscriptionResponse
from pydantic import BaseModel, JsonValue, TypeAdapter

Object: TypeAlias = dict[str, JsonValue]
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EMPTY: Object = {}
REQUEST = httpx.Request("POST", "https://localhost")


class Urls(Protocol):
    def get_complete_url(
        self, base: str | None, key: str | None, model: str, params: Object, extra: Object, /
    ) -> str: ...


class EmbedConfig(Urls, Protocol):
    def transform_embedding_request(
        self, model: str, value: AllEmbeddingInputValues, params: Object, headers: Object, /
    ) -> Object: ...

    @property
    def transform_embedding_response(self) -> Callable[
        [str, httpx.Response, EmbeddingResponse, Logging, str | None, Object, Object, Object],
        EmbeddingResponse,
    ]: ...


class RerankConfig(Protocol):
    @property
    def map_cohere_rerank_params(self) -> Callable[[Object, str, bool, str, list[str]], Object]: ...

    @property
    def validate_environment(self) -> Callable[
        [dict[str, str], str, str | None, Object], dict[str, str]
    ]: ...

    def transform_rerank_request(
        self, model: str, params: Object, headers: Object, /
    ) -> Object: ...

    @property
    def transform_rerank_response(self) -> Callable[
        [str, httpx.Response, RerankResponse, Logging], RerankResponse
    ]: ...


class AudioConfig(Urls, Protocol):
    # the request dataclass holds a dict or bytes, so the form comes back through a view
    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self, model: str, audio: tuple[str, bytes, str], params: Object, extra: Object, /
    ) -> AudioTranscriptionRequestData: ...

    def transform_audio_transcription_response(  # ast-grep-ignore: no-long-names
        self, raw: httpx.Response, /
    ) -> TranscriptionResponse: ...


class PassthroughConfig(Protocol):
    @property
    def get_complete_url(self) -> Callable[
        [str | None, str | None, str, str, Object | None, Object], tuple[httpx.URL, str]
    ]: ...

    def get_api_key(self, api_key: str | None, /) -> str | None: ...

    def is_streaming_request(self, endpoint: str, data: Object, /) -> bool: ...


class AudioRequestView(BaseModel):
    data: Object


class AudioFile(TypedDict):
    filename: str
    content_type: str
    content: str


class EmbedStep(TypedDict):
    model: str
    input: AllEmbeddingInputValues
    options: Object
    api_base: str | None
    reply: str | None


class RerankStep(TypedDict):
    model: str
    options: Object
    headers: dict[str, str]
    api_key: str | None
    reply: str | None
    status: int | None


class AudioStep(TypedDict):
    model: str
    options: Object
    api_base: str | None
    file: AudioFile | None
    reply: str | None


class PassthroughStep(TypedDict):
    api_base: str | None
    api_key: str | None
    endpoint: str
    query: Object | None
    data: Object


class Input(TypedDict):
    family: Literal["embed", "rerank", "audio", "passthrough"]
    env: dict[str, str]
    step: JsonValue


CONVERTED: dict[str, bool | dict[str, bool]] = {
    "object": True,
    "data": True,
    "usage": {"prompt_tokens": True, "completion_tokens": True, "total_tokens": True},
}
# BaseConfig.__init__ only passes, and __new__ keeps mypy from reading the untyped call
EMBED: EmbedConfig = IBMWatsonXEmbeddingConfig.__new__(IBMWatsonXEmbeddingConfig)
RERANK: RerankConfig = IBMWatsonXRerankConfig()
AUDIO: AudioConfig = WatsonxAudio.__new__(WatsonxAudio)
PASSTHROUGH: PassthroughConfig = WatsonxPassthroughConfig()
LOGGING = Logging(
    model="model", messages=None, stream=False, call_type="embedding",
    start_time=datetime(2024, 1, 1, tzinfo=UTC), litellm_call_id="call", function_id="function",
)


def embed(step: EmbedStep) -> JsonValue:
    options = dict(step["options"])
    url = EMBED.get_complete_url(step["api_base"], None, step["model"], options, EMPTY)
    body = EMBED.transform_embedding_request(step["model"], step["input"], options, EMPTY)
    reply: JsonValue = None
    if (text := step["reply"]) is not None:
        raw = httpx.Response(200, content=text.encode(), request=REQUEST)
        response = EMBED.transform_embedding_response(
            step["model"], raw, EmbeddingResponse(object="list"), LOGGING, None, EMPTY, EMPTY, EMPTY
        )
        # model and usage details come from the caller's response, so only converted fields compare
        reply = JSON.validate_json(response.model_dump_json(include=CONVERTED))
    return {"url": url, "request": body, "reply": reply}


def rerank(step: RerankStep) -> JsonValue:
    options = dict(step["options"])
    mapped = RERANK.map_cohere_rerank_params(options, step["model"], False, "", [])
    auth_options = dict(options)
    headers = RERANK.validate_environment(
        dict(step["headers"]), step["model"], step["api_key"], auth_options
    )
    request = RERANK.transform_rerank_request(step["model"], dict(mapped), EMPTY)
    reply: JsonValue = None
    if (text := step["reply"]) is not None and (status := step["status"]) is not None:
        raw = httpx.Response(status, content=text.encode(), request=REQUEST)
        reply = JSON.validate_json(
            RERANK.transform_rerank_response(
                step["model"], raw, RerankResponse(), LOGGING
            ).model_dump_json()
        )
    return {
        "map": mapped,
        "auth": {
            "headers": dict(headers),
            "options": auth_options,
        },
        "request": request,
        "reply": reply,
    }


def audio(step: AudioStep) -> JsonValue:
    options = dict(step["options"])
    url = AUDIO.get_complete_url(step["api_base"], None, step["model"], options, EMPTY)
    request: JsonValue = None
    if (audio_file := step["file"]) is not None:
        content = audio_file["content"].encode()
        data = AUDIO.transform_audio_transcription_request(
            step["model"],
            (audio_file["filename"], content, audio_file["content_type"]),
            options,
            EMPTY,
        )
        form = AudioRequestView.model_validate(data, from_attributes=True)
        request = {
            "data": form.data,
            "file": {
                "filename": audio_file["filename"],
                "content_type": audio_file["content_type"],
                "size": len(content),
            },
        }
    reply: JsonValue = None
    if (text := step["reply"]) is not None:
        reply = JSON.validate_json(
            AUDIO.transform_audio_transcription_response(
                httpx.Response(200, content=text.encode(), request=REQUEST)
            ).model_dump_json()
        )
    return {"url": url, "request": request, "reply": reply}


def passthrough(step: PassthroughStep) -> JsonValue:
    url, base = PASSTHROUGH.get_complete_url(
        step["api_base"], None, "model", step["endpoint"], step["query"], EMPTY
    )
    key = PASSTHROUGH.get_api_key(step["api_key"])
    return {
        "url": {"url": str(url), "base": base},
        "stream": PASSTHROUGH.is_streaming_request(step["endpoint"], dict(step["data"])),
        "key": key,
    }


def run(input_value: Input) -> JsonValue:
    os.environ.update(input_value["env"])
    step = input_value["step"]
    if input_value["family"] == "embed":
        return embed(TypeAdapter(EmbedStep).validate_python(step))
    if input_value["family"] == "rerank":
        return rerank(TypeAdapter(RerankStep).validate_python(step))
    if input_value["family"] == "audio":
        return audio(TypeAdapter(AudioStep).validate_python(step))
    return passthrough(TypeAdapter(PassthroughStep).validate_python(step))
