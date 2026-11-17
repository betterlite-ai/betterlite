from typing import Final

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.embedding.transformation import BaseEmbeddingConfig
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

from .configs import COHERE, CONFIG_NAMES, HF, LOG, LOG_START, MODEL
from .types import (
    CohereMap,
    CohereReply,
    CohereRequest,
    Headers,
    HfReply,
    HfRequest,
    ModelConfig,
    Object,
)
from .views import COHERE_REPLY, HF_REPLY, usage_value

MODEL_CONFIG: Final = TypeAdapter(ModelConfig)
SUPPORTED: Final = TypeAdapter(list[JsonValue])
HEADERS: Final = TypeAdapter(Headers)
HF_REQUEST: Final = TypeAdapter(HfRequest)
HF_REPLY_INPUT: Final = TypeAdapter(HfReply)
COHERE_MAP_INPUT: Final = TypeAdapter(CohereMap)
COHERE_REQUEST_INPUT: Final = TypeAdapter(CohereRequest)
COHERE_REPLY_INPUT: Final = TypeAdapter(CohereReply)


def model_config(input_value: dict[str, JsonValue]) -> JsonValue:
    case = MODEL_CONFIG.validate_python(input_value)
    config = HF.get_model_config(case["model"])
    supported = SUPPORTED.validate_python(HF.get_supported_openai_params(case["model"]))
    return {"config": CONFIG_NAMES[type(config).__name__], "supported": supported}


def headers(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HEADERS.validate_python(input_value)
    config: BaseEmbeddingConfig = HF if case["kind"] == "hugging_face" else COHERE
    return {
        "kind": case["kind"],
        "headers": config.validate_environment(case["headers"], MODEL, [], {}, {}),
        "given": case["headers"],
    }


def hf_request(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HF_REQUEST.validate_python(input_value)
    return {
        "request": HF.transform_embedding_request(
            MODEL, case["input"], case["optional_params"], {}
        )
    }


def hf_reply(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HF_REPLY_INPUT.validate_python(input_value)
    raw = httpx.Response(case["status"], content=case["body"].encode())
    response = HF.transform_embedding_response(
        case["model"], raw, EmbeddingResponse(), LOG, None, case["request"], {}, {}
    )
    reply = HF_REPLY.validate_json(response.model_dump_json())
    rows: list[JsonValue] = [
        {"object": row["object"], "index": row["index"], "embedding": row["embedding"]}
        for row in reply["data"]
    ]
    return {
        "object": reply["object"],
        "data": rows,
        "model": reply["model"],
        "usage": usage_value(reply["usage"]),
    }


def cohere_map(input_value: dict[str, JsonValue]) -> JsonValue:
    case = COHERE_MAP_INPUT.validate_python(input_value)
    return {
        "mapped": COHERE.map_openai_params(
            case["non_default"], case["optional_params"], MODEL, drop_params=False
        )
    }


def cohere_request(input_value: dict[str, JsonValue]) -> JsonValue:
    case = COHERE_REQUEST_INPUT.validate_python(input_value)
    return {
        "request": COHERE.transform_embedding_request(
            case["model"], case["input"], case["optional_params"], {}
        )
    }


def cohere_reply(input_value: dict[str, JsonValue]) -> JsonValue:
    case = COHERE_REPLY_INPUT.validate_python(input_value)
    raw = httpx.Response(200, content=case["body"].encode())
    logging_obj = Logging(
        model=MODEL,
        messages=case["input"],
        stream=False,
        call_type="embeddings",
        start_time=LOG_START,
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    response = COHERE.transform_embedding_response(
        case["model"], raw, EmbeddingResponse(), logging_obj, None, case["request_data"], {}, {}
    )
    reply = COHERE_REPLY.validate_json(response.model_dump_json())
    rows: list[JsonValue] = []
    for row in reply["data"]:
        value: Object = {
            "object": row["object"],
            "index": row["index"],
            "embedding": row["embedding"],
        }
        kind = row.get("type")
        if kind is not None:
            value["type"] = kind
        rows.append(value)
    return {
        "object": reply["object"],
        "data": rows,
        "model": reply["model"],
        "usage": usage_value(reply["usage"]),
    }
