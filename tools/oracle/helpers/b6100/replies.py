from typing import Literal

from litellm.types.utils import CallTypes, EmbeddingResponse, ModelResponse
from pydantic import JsonValue

from .configs import CHAT, EMBEDDING, HANDLER, MESSAGES, RESPONSES, logging_object, raw_reply
from .views import ChunkRecord, RecordedLogging, hidden_cost
from .wire import MODEL, Object


def transformed_reply(body: str, family: Literal["chat", "embedding"]) -> JsonValue:
    reply: ModelResponse | EmbeddingResponse
    if family == "chat":
        reply = CHAT.transform_response(
            MODEL,
            raw_reply(body),
            ModelResponse(),
            logging_object(CallTypes.completion),
            {},
            [],
            {},
            {},
            None,
        )
    else:
        reply = EMBEDDING.transform_embedding_response(
            MODEL,
            raw_reply(body),
            EmbeddingResponse(),
            logging_object(CallTypes.embedding),
            None,
            {},
            {},
            {},
        )
    return {"cost": hidden_cost(reply)}


def chunk_value(chunk: Object) -> JsonValue:
    record = ChunkRecord.model_validate(HANDLER.chunk_parser(chunk), from_attributes=True)
    if record.usage is None:
        return {"cost": None}
    return {"cost": record.usage.cost}


def responses_reply_value(body: str) -> JsonValue:
    reply = RESPONSES.transform_response_api_response(
        MODEL, raw_reply(body), logging_object(CallTypes.responses)
    )
    return {"cost": hidden_cost(reply)}


def messages_reply_value(body: str) -> JsonValue:
    logging_obj = logging_object(CallTypes.anthropic_messages)
    MESSAGES.transform_anthropic_messages_response(MODEL, raw_reply(body), logging_obj)
    details = RecordedLogging.model_validate(logging_obj, from_attributes=True)
    return {"cost": details.model_call_details.response_cost}
