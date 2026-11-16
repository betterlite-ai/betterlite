import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.watsonx.audio_transcription.transformation import (
    IBMWatsonXAudioTranscriptionConfig,
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
POST = "https://localhost"
START = datetime(2024, 1, 1, tzinfo=UTC)


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
    def map_cohere_rerank_params(self) -> Callable[
        [Object, str, bool, str, list[str]], Object
    ]: ...

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
    # the request dataclass holds a dict or bytes, so the form data comes back through the view below
    def transform_audio_transcription_request(  # ast-grep-ignore: no-long-names
        self, model: str, audio: tuple[str, bytes, str], params: Object, extra: Object, /
    ) -> object: ...

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


class EmbedItem(TypedDict):
    object: str
    index: int
    embedding: JsonValue


class EmbedUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class EmbedReply(TypedDict):
    object: str
    data: list[EmbedItem]
    usage: EmbedUsage


class Step(TypedDict, total=False):
    model: str
    input: AllEmbeddingInputValues
    options: Object
    headers: dict[str, str]
    api_base: str | None
    api_key: str | None
    file: dict[str, str] | None
    reply: str | None
    status: int | None
    endpoint: str
    query: Object | None
    data: Object


class Input(TypedDict):
    family: Literal["embed", "rerank", "audio", "passthrough"]
    env: dict[str, str]
    step: JsonValue


STEP: TypeAdapter[Step] = TypeAdapter(Step)
EMBED_REPLY: TypeAdapter[EmbedReply] = TypeAdapter(EmbedReply)
EMBED: EmbedConfig = IBMWatsonXEmbeddingConfig()
RERANK: RerankConfig = IBMWatsonXRerankConfig()
AUDIO: AudioConfig = IBMWatsonXAudioTranscriptionConfig()
PASSTHROUGH: PassthroughConfig = WatsonxPassthroughConfig()
LOGGING = Logging(
    model="model", messages=None, stream=False, call_type="embedding",
    start_time=START, litellm_call_id="call", function_id="function",
)


def posted(body: str, status: int = 200) -> httpx.Response:
    return httpx.Response(status, content=body.encode(), request=httpx.Request("POST", POST))


def embed(step: Step) -> JsonValue:
    options = dict(step["options"])
    url = EMBED.get_complete_url(step["api_base"], None, step["model"], options, EMPTY)
    body = EMBED.transform_embedding_request(step["model"], step["input"], options, EMPTY)
    reply: JsonValue = None
    if (text := step["reply"]) is not None:
        response = EMBED.transform_embedding_response(
            step["model"],
            posted(text),
            EmbeddingResponse(object="list"),
            LOGGING,
            None,
            EMPTY,
            EMPTY,
            EMPTY,
        )
        # the model and usage detail fields depend on the caller's response object, so only the converted fields are compared
        reply = JSON.validate_json(
            EMBED_REPLY.dump_json(EMBED_REPLY.validate_json(response.model_dump_json()))
        )
    return {"url": url, "request": body, "reply": reply}


def rerank(step: Step) -> JsonValue:
    options = dict(step["options"])
    mapped = RERANK.map_cohere_rerank_params(options, step["model"], False, "", [])
    auth_options = dict(options)
    headers = RERANK.validate_environment(
        dict(step["headers"]), step["model"], step["api_key"], auth_options
    )
    request = RERANK.transform_rerank_request(step["model"], dict(mapped), EMPTY)
    reply: JsonValue = None
    if (text := step["reply"]) is not None and (status := step["status"]) is not None:
        reply = JSON.validate_json(
            RERANK.transform_rerank_response(
                step["model"], posted(text, status), RerankResponse(), LOGGING
            ).model_dump_json()
        )
    return {
        "map": mapped,
        "auth": {
            "headers": {name.lower(): value for name, value in headers.items()},
            "options": auth_options,
        },
        "request": request,
        "reply": reply,
    }


def audio(step: Step) -> JsonValue:
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
            AUDIO.transform_audio_transcription_response(posted(text)).model_dump_json()
        )
    return {"url": url, "request": request, "reply": reply}


def passthrough(step: Step) -> JsonValue:
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
    step = STEP.validate_python(input_value["step"])
    if input_value["family"] == "embed":
        return embed(step)
    if input_value["family"] == "rerank":
        return rerank(step)
    if input_value["family"] == "audio":
        return audio(step)
    return passthrough(step)
