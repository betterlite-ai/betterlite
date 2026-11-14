from typing import TypeVar

import httpx
from litellm.types.utils import EmbeddingResponse, ModelResponse
from pydantic import JsonValue, TypeAdapter

from .steps import ChoicesChunk, EmbeddingResponseStep, EventChunk, Object, ResponseStep
from .views import (
    ChatView,
    EmbeddingReplyView,
    EmbeddingView,
    EventView,
    OpenAiChunkView,
    Recorder,
    Reply,
    ReplyView,
    StreamHandler,
)

Shape = TypeVar("Shape")


def shaped(adapter: TypeAdapter[Shape], source: str | bytes) -> JsonValue:
    view = adapter.validate_json(source)
    return TypeAdapter(JsonValue).validate_json(adapter.dump_json(view))


def reply_fields(adapter: TypeAdapter[Shape], reply: Reply) -> Object:
    return {
        "reply": shaped(adapter, reply.model_dump_json()),
        "hidden_model": reply.get("_hidden_params")["model"],
    }


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
    return {**reply_fields(TypeAdapter(ReplyView), reply), "logged": recorder.calls}


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
    return reply_fields(TypeAdapter(EmbeddingReplyView), reply)


def chunk_step(handler: StreamHandler, chunk: ChoicesChunk) -> JsonValue:
    adapter = TypeAdapter(OpenAiChunkView)
    return {"chunk": shaped(adapter, adapter.dump_json(handler.chunk_parser(chunk)))}


def event_step(handler: StreamHandler, chunk: EventChunk) -> JsonValue:
    parsed = handler.chunk_parser(chunk)
    return {"chunk": shaped(TypeAdapter(EventView), parsed.model_dump_json())}
