import datetime as dt
import os
from collections.abc import Callable
from types import ModuleType
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.voyage.embedding.transformation_multimodal import (
    VoyageMultimodalEmbeddingConfig as MultimodalImpl,
)
from litellm.llms.voyage.rerank import transformation as rerank_transformation
from litellm.llms.voyage.rerank.transformation import VoyageRerankConfig as RerankImpl
from litellm.types.llms.openai import AllEmbeddingInputValues
from litellm.types.rerank import RerankResponse
from litellm.types.utils import EmbeddingResponse, ModelInfo
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JsonObject: TypeAlias = dict[str, JsonValue]
EMPTY: Final[JsonObject] = {}
ALIASES: Final = ("VOYAGE_API_KEY", "VOYAGE_AI_API_KEY", "VOYAGE_AI_TOKEN")


class MultimodalConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[JsonValue]]
    map_openai_params: Callable[..., JsonObject]
    transform_embedding_request: Callable[
        [str, AllEmbeddingInputValues, JsonObject, JsonObject], JsonObject
    ]


class RerankConfig(Protocol):
    get_supported_cohere_rerank_params: Callable[[str], list[JsonValue]]
    map_cohere_rerank_params: Callable[..., JsonObject]
    transform_rerank_request: Callable[..., JsonObject]
    validate_environment: Callable[..., JsonObject]


class MultimodalConfigImpl(MultimodalImpl):
    def __init__(self) -> None:
        pass


MULTIMODAL_IMPL: Final = MultimodalConfigImpl()
MULTIMODAL: Final[MultimodalConfig] = MULTIMODAL_IMPL
RERANK_IMPL: Final = RerankImpl()
RERANK: Final[RerankConfig] = RERANK_IMPL


class Reply(TypedDict):
    status: int
    body: str


class MultimodalCall(TypedDict):
    model: str
    input: AllEmbeddingInputValues
    options: JsonObject
    base: str | None
    api_key: str | None
    env: dict[str, str]
    reply: Reply


class ModelInfoInput(TypedDict):
    input_cost_per_token: float | None


class RerankCall(TypedDict):
    model: str
    query: str
    documents: list[str | dict[str, JsonValue]]
    top_n: int | None
    return_documents: bool | None
    rank_fields: list[str] | None
    max_chunks_per_doc: int | None
    max_tokens_per_doc: int | None
    instruction: str | None
    base: str | None
    api_key: str | None
    env: dict[str, str]
    headers: JsonObject
    reply: Reply
    model_info: ModelInfoInput | None
    uuid: str


class Input(TypedDict):
    multimodal: list[MultimodalCall]
    rerank: list[RerankCall]


class EmbeddingUsage(TypedDict):
    prompt_tokens: int
    total_tokens: int


class EmbeddingReply(TypedDict):
    model: JsonValue
    data: JsonValue
    object: JsonValue
    usage: EmbeddingUsage


class BilledUnits(TypedDict):
    total_tokens: int | None


class Tokens(TypedDict):
    input_tokens: int | None
    output_tokens: int


class Meta(TypedDict):
    billed_units: BilledUnits
    tokens: Tokens


class ResultProjection(TypedDict):
    index: int
    relevance_score: float
    document: NotRequired[JsonValue]


class RerankProjection(TypedDict):
    id: str
    results: list[ResultProjection]
    meta: Meta


class FixedUuid(ModuleType):
    fixed: str

    def __init__(self, fixed: str) -> None:
        super().__init__("litellm._uuid")
        self.fixed = fixed

    def uuid4(self) -> str:
        return self.fixed


class RecordingLogging(Logging):
    calls: list[str]

    def __init__(self) -> None:
        super().__init__(
            model="voyage-rerank",
            messages=None,
            stream=False,
            call_type="rerank",
            start_time=dt.datetime.now(dt.UTC),
            litellm_call_id="voyage-media-rank",
            function_id="voyage-media-rank",
        )
        self.calls = []

    @override
    def post_call(
        self,
        original_response: str,
        input: JsonValue = None,
        api_key: JsonValue = None,
        additional_args: JsonValue = None,
    ) -> None:
        del input, api_key, additional_args
        self.calls.append(original_response)


def apply_env(environment: dict[str, str]) -> None:
    for name in ALIASES:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def json_number(value: float) -> int | float:
    if value.is_integer():
        return int(value)
    return value


def multimodal_result(call: MultimodalCall) -> JsonValue:
    apply_env(call["env"])
    try:
        request: JsonValue = {
            "ok": True,
            "value": MULTIMODAL.transform_embedding_request(
                call["model"],
                call["input"],
                MULTIMODAL.map_openai_params(
                    call["options"],
                    {
                        name: value
                        for name, value in call["options"].items()
                        if name != "dimensions"
                    },
                    call["model"],
                    drop_params=False,
                ),
                EMPTY,
            ),
        }
    except ValueError:
        request = {"ok": False, "status": None}
    raw = httpx.Response(call["reply"]["status"], content=call["reply"]["body"])
    try:
        answer = MULTIMODAL_IMPL.transform_embedding_response(
            call["model"],
            raw,
            EmbeddingResponse(),
            Logging(
                model=call["model"],
                messages=None,
                stream=False,
                call_type="embedding",
                start_time=dt.datetime.now(dt.UTC),
                litellm_call_id="voyage-media-rank",
                function_id="voyage-media-rank",
            ),
            api_key=None,
            request_data=EMPTY,
            optional_params=EMPTY,
            litellm_params=EMPTY,
        )
        fields = TypeAdapter(EmbeddingReply).validate_json(answer.model_dump_json())
        response: JsonValue = {
            "ok": True,
            "value": {
                "model": fields["model"],
                "data": fields["data"],
                "object": fields["object"],
                "usage": {
                    "prompt_tokens": fields["usage"]["prompt_tokens"],
                    "total_tokens": fields["usage"]["total_tokens"],
                },
            },
        }
    except BaseLLMException as error:
        response = {"ok": False, "status": error.status_code}
    return {
        "is_multimodal": MultimodalImpl.is_multimodal_embeddings(call["model"]),
        "supported_params": MULTIMODAL.get_supported_openai_params(call["model"]),
        "url": MULTIMODAL_IMPL.get_complete_url(
            call["base"], call["api_key"], call["model"], EMPTY, EMPTY
        ),
        "request": request,
        "response": response,
    }


def rerank_model_info(price: float | None) -> ModelInfo | None:
    if price is None:
        return None
    return ModelInfo(
        key="voyage-rerank",
        max_tokens=None,
        max_input_tokens=None,
        max_output_tokens=None,
        input_cost_per_token=price,
        output_cost_per_token=None,
        litellm_provider="voyage",
        mode="completion",
        supported_openai_params=None,
    )


def rerank_reply_fields(response: RerankResponse) -> tuple[JsonValue, BilledUnits]:
    reply = TypeAdapter(RerankProjection).validate_json(response.model_dump_json())
    results: list[JsonValue] = []
    for result in reply["results"]:
        item: JsonObject = {
            "index": result["index"],
            "relevance_score": json_number(result["relevance_score"]),
        }
        if "document" in result:
            item["document"] = result["document"]
        results.append(item)
    billed = reply["meta"]["billed_units"]
    tokens = reply["meta"]["tokens"]
    value: JsonValue = {
        "id": reply["id"],
        "results": results,
        "meta": {
            "billed_units": {"total_tokens": billed["total_tokens"]},
            "tokens": {
                "input_tokens": tokens["input_tokens"],
                "output_tokens": tokens["output_tokens"],
            },
        },
    }
    return value, billed


def rerank_result(call: RerankCall) -> JsonValue:
    apply_env(call["env"])
    try:
        headers: JsonValue = {
            "ok": True,
            "value": RERANK.validate_environment(
                call["headers"], call["model"], api_key=call["api_key"]
            ),
        }
    except ValueError:
        headers = {"ok": False, "status": None}
    try:
        request: JsonValue = {
            "ok": True,
            "value": RERANK.transform_rerank_request(
                call["model"],
                RERANK.map_cohere_rerank_params(
                    EMPTY,
                    call["model"],
                    drop_params=False,
                    query=call["query"],
                    documents=call["documents"],
                    top_n=call["top_n"],
                    rank_fields=call["rank_fields"],
                    return_documents=call["return_documents"],
                    max_chunks_per_doc=call["max_chunks_per_doc"],
                    max_tokens_per_doc=call["max_tokens_per_doc"],
                    instruction=call["instruction"],
                ),
                EMPTY,
                None,
            ),
        }
    except ValueError:
        request = {"ok": False, "status": None}
    ModuleType.__setattr__(rerank_transformation, "uuid", FixedUuid(call["uuid"]))
    raw = httpx.Response(call["reply"]["status"], content=call["reply"]["body"])
    logging = RecordingLogging()
    billed: BilledUnits | None = None
    try:
        answer = RERANK_IMPL.transform_rerank_response(
            call["model"],
            raw,
            RerankResponse(),
            logging,
            api_key=None,
            request_data=EMPTY,
            optional_params=EMPTY,
            litellm_params=EMPTY,
        )
        value, billed = rerank_reply_fields(answer)
        response: JsonValue = {"ok": True, "value": value}
    except BaseLLMException as error:
        response = {"ok": False, "status": error.status_code}
    except ValueError:
        response = {"ok": False, "status": None}
    info = rerank_model_info(
        None if call["model_info"] is None else call["model_info"]["input_cost_per_token"]
    )
    total, completion = RERANK_IMPL.calculate_rerank_cost(call["model"], None, billed, info)
    return {
        "supported_params": RERANK.get_supported_cohere_rerank_params(call["model"]),
        "headers": headers,
        "url": RERANK_IMPL.get_complete_url(call["base"], call["model"], None),
        "request": request,
        "response": response,
        "post_call": bool(logging.calls),
        "cost": [json_number(total), json_number(completion)],
    }


def run(input_value: Input) -> JsonValue:
    return {
        "multimodal": [multimodal_result(call) for call in input_value["multimodal"]],
        "rerank": [rerank_result(call) for call in input_value["rerank"]],
    }
