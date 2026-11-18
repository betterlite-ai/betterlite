from typing import Final, override

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.embedding.transformation import BaseEmbeddingConfig
from litellm.llms.cohere.embed.v1_transformation import _SupportsEncode  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cohere/embed/v1_transformation.py#L20-L23
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue, TypeAdapter

from .configs import COHERE, CONFIG_NAMES, HF, LOG, LOG_START, MODEL
from .types import (
    COHERE_REPLY,
    HF_REPLY,
    CohereMap,
    CohereReply,
    CohereRequest,
    Headers,
    HfReply,
    HfRequest,
    ModelConfig,
    Object,
    usage_value,
)

MODEL_CONFIG: Final = TypeAdapter(ModelConfig)
SUPPORTED: Final = TypeAdapter(list[JsonValue])
HEADERS: Final = TypeAdapter(Headers)
HF_REQUEST: Final = TypeAdapter(HfRequest)
HF_REPLY_INPUT: Final = TypeAdapter(HfReply)
COHERE_MAP_INPUT: Final = TypeAdapter(CohereMap)
COHERE_REQUEST_INPUT: Final = TypeAdapter(CohereRequest)
COHERE_REPLY_INPUT: Final = TypeAdapter(CohereReply)
OBJECT: Final = TypeAdapter(Object)


def model_config(input_value: dict[str, JsonValue]) -> JsonValue:
    case = MODEL_CONFIG.validate_python(input_value)
    config = HF.get_model_config(case["model"])
    supported = SUPPORTED.validate_python(HF.get_supported_openai_params(case["model"]))
    return {"config": CONFIG_NAMES[type(config).__name__], "supported": supported}


def headers(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HEADERS.validate_python(input_value)
    config: BaseEmbeddingConfig = HF if case["kind"] == "hugging_face" else COHERE
    empty: Object = {}
    validated: Object = OBJECT.validate_python(
        config.validate_environment(case["headers"], MODEL, [], empty, empty)  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/transformation.py#L146-L159
    )
    given: Object = OBJECT.validate_python(case["headers"])
    return {"kind": case["kind"], "headers": validated, "given": given}


def hf_request(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HF_REQUEST.validate_python(input_value)
    return {
        "request": HF.transform_embedding_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/transformation.py#L70-L81
            MODEL, case["input"], case["optional_params"], {}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/transformation.py#L70-L81
        )
    }


def hf_reply(input_value: dict[str, JsonValue]) -> JsonValue:
    case = HF_REPLY_INPUT.validate_python(input_value)
    raw = httpx.Response(case["status"], content=case["body"].encode())
    empty: Object = {}
    response = HF.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/transformation.py#L83-L100
        case["model"], raw, EmbeddingResponse(), LOG, None, case["request"], empty, empty
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
        "mapped": COHERE.map_openai_params(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/cohere_transformation.py#L44-L57
            case["non_default"], case["optional_params"], MODEL, drop_params=False
        )
    }


def cohere_request(input_value: dict[str, JsonValue]) -> JsonValue:
    case = COHERE_REQUEST_INPUT.validate_python(input_value)
    return {
        "request": COHERE.transform_embedding_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/cohere_transformation.py#L62-L87
            case["model"], case["input"], case["optional_params"], {}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/cohere_transformation.py#L62-L87
        )
    }


# LiteLLM counts unbilled usage with litellm.encoding; counting characters matches the Rust helper
class Encoding(_SupportsEncode):
    @override
    def encode(self, text: str, /) -> list[int]:
        return list(range(len(text)))


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
    empty: Object = {}
    litellm.encoding = Encoding()  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/cohere_transformation.py#L118
    response = COHERE.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/embedding/cohere_transformation.py#L89-L120
        case["model"],
        raw,
        EmbeddingResponse(),
        logging_obj,
        None,
        case["request_data"],
        empty,
        empty,
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
