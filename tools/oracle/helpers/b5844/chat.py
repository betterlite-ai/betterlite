from typing import TypeVar

import httpx
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter

from .views import (
    ChatView,
    ChoicesChunk,
    EmbeddingReplyView,
    EmbeddingRequestStep,
    EmbeddingResponseStep,
    EmbeddingView,
    EventView,
    Object,
    OpenAiChunkView,
    Recorder,
    ReplyView,
    RequestStep,
    ResponseStep,
    StreamHandler,
)

Shape = TypeVar("Shape")


def shaped(adapter: TypeAdapter[Shape], source: str | bytes) -> JsonValue:
    view = adapter.validate_json(source)
    return TypeAdapter(JsonValue).validate_json(adapter.dump_json(view))


def request_step(chat: ChatView, step: RequestStep) -> JsonValue:
    body = chat.transform_request(
        model=step["model"],
        messages=step["messages"],
        optional_params=dict(step["options"]),
        litellm_params={},
        headers={},
    )
    return {"body": body}


def response_step(chat: ChatView, step: ResponseStep) -> JsonValue:
    recorder = Recorder()
    reply = chat.transform_response(
        model=step["model"],
        raw_response=httpx.Response(200, content=step["body"].encode()),
        model_response=ModelResponse(),
        logging_obj=recorder,
        request_data=step["request_data"],
        messages=step["messages"],
        optional_params={},
        litellm_params={},
        encoding=None,
    )
    return {
        "reply": shaped(TypeAdapter(ReplyView), reply.model_dump_json()),
        "hidden_model": reply.get("_hidden_params")["model"],
        "logged": recorder.calls,
    }


def embedding_request_step(embed: EmbeddingView, step: EmbeddingRequestStep) -> JsonValue:
    body = embed.transform_embedding_request(
        model=step["model"],
        input=step["text"],
        optional_params=dict(step["options"]),
        headers={},
    )
    return {"body": body}


def embedding_response_step(embed: EmbeddingView, step: EmbeddingResponseStep) -> JsonValue:
    reply = embed.transform_embedding_response(
        model=step["model"],
        raw_response=httpx.Response(200, content=step["body"].encode()),
        model_response=EmbeddingResponse(),
        logging_obj=None,
        api_key=None,
        request_data={},
        optional_params={},
        litellm_params={},
    )
    return {
        "reply": shaped(TypeAdapter(EmbeddingReplyView), reply.model_dump_json()),
        "hidden_model": reply.get("_hidden_params")["model"],
    }


def chunk_step(handler: StreamHandler, chunk: ChoicesChunk) -> JsonValue:
    adapter = TypeAdapter(OpenAiChunkView)
    return {"chunk": shaped(adapter, adapter.dump_json(handler.chunk_parser(chunk)))}


def event_step(handler: StreamHandler, chunk: Object) -> JsonValue:
    parsed = handler.chunk_parser(chunk)
    return {"chunk": shaped(TypeAdapter(EventView), parsed.model_dump_json())}
