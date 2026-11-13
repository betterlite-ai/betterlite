from decimal import Decimal
from typing import Final, Literal, TypedDict, cast

import litellm
from litellm.llms.vertex_ai.batches.transformation import (
    VertexAIBatchTransformation as BatchTransform,
)
from litellm.llms.vertex_ai.batches.transformation import (
    native_vertex_batch_row_stats,
)
from litellm.llms.vertex_ai.common_utils import VertexAIError
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import (
    VertexGeminiConfig,
)
from litellm.types.llms.openai import CreateBatchRequest
from litellm.types.llms.vertex_ai import VertexBatchPredictionResponse
from litellm.types.utils import LiteLLMBatch, ModelInfo, Usage
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)


class CreateCall(TypedDict):
    data: CreateBatchRequest
    vertex_project: str | None
    vertex_location: str | None


class RowCall(TypedDict):
    row: dict[str, JsonValue]
    model_name: str | None
    model_info: dict[str, JsonValue] | None
    pricing: Literal["fixed", "requires_info", "failure"]


class ListPage(TypedDict):
    object: str
    data: list[LiteLLMBatch]
    first_id: str | None
    last_id: str | None
    has_more: bool
    next_page_token: str | None


class CreateInput(TypedDict):
    operation: Literal["create"]
    calls: list[CreateCall]


class JobInput(TypedDict):
    operation: Literal["job"]
    calls: list[VertexBatchPredictionResponse]


class ListInput(TypedDict):
    operation: Literal["list"]
    calls: list[dict[str, JsonValue]]


class RowInput(TypedDict):
    operation: Literal["row"]
    calls: list[RowCall]


Input = CreateInput | JobInput | ListInput | RowInput


class FixturePricing:
    def __init__(self, pricing: Literal["fixed", "requires_info", "failure"]) -> None:
        self.pricing = pricing

    def __call__(
        self,
        usage: Usage,
        model: str,
        custom_llm_provider: str | None = None,
        model_info: ModelInfo | None = None,
    ) -> tuple[float, float]:
        del usage, model
        missing = self.pricing == "requires_info" and model_info is None  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
        if missing or self.pricing == "failure" or custom_llm_provider != "vertex_ai":
            raise ValueError(custom_llm_provider)
        return (0.25, 0.5)


def job_state(batch: LiteLLMBatch) -> JsonValue:
    return {
        "id": batch.id,
        "completion_window": batch.completion_window,
        "created_at": batch.created_at,
        "endpoint": batch.endpoint,
        "input_file_id": batch.input_file_id,
        "object": batch.object,
        "status": batch.status,
        "error_file_id": batch.error_file_id,
        "output_file_id": batch.output_file_id,
    }


def create_outcome(call: CreateCall) -> JsonValue:
    try:
        job = BatchTransform.transform_openai_batch_request_to_vertex_ai_batch_request(
            call["data"],
            vertex_project=call["vertex_project"],
            vertex_location=call["vertex_location"],
        )
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        return {"ok": False, "status": error.status_code}
    except (TypeError, ValueError):
        return {"ok": False, "status": None}
    return {"ok": True, "job": JSON.validate_python(job)}


def job_outcome(response: VertexBatchPredictionResponse) -> JsonValue:
    try:
        batch = BatchTransform.transform_vertex_ai_batch_response_to_openai_batch_response(response)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {"ok": True, "batch": job_state(batch)}


def list_outcome(response: dict[str, JsonValue]) -> JsonValue:
    try:
        page = TypeAdapter(ListPage).validate_python(
            BatchTransform.transform_vertex_ai_batch_list_response_to_openai_list_response(response)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {
        "ok": True,
        "page": {
            "object": page["object"],
            "data": [job_state(batch) for batch in page["data"]],
            "first_id": page["first_id"],
            "last_id": page["last_id"],
            "has_more": page["has_more"],
            "next_page_token": page["next_page_token"],
        },
    }


def row_outcome(call: RowCall) -> JsonValue:
    stats = native_vertex_batch_row_stats(
        call["row"],
        call["model_name"],
        model_info=cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
            "ModelInfo | None", call["model_info"]
        ),
        calculate_usage=VertexGeminiConfig._calculate_usage,  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1742
        cost_calculator=FixturePricing(call["pricing"]),
    )
    if stats is None:
        return {"ok": True, "stats": None}
    return {
        "ok": True,
        "stats": {
            "usage": {
                "prompt_tokens": stats.usage.prompt_tokens,
                "completion_tokens": stats.usage.completion_tokens,
                "total_tokens": stats.usage.total_tokens,
            },
            "total_tokens": stats.total_tokens,
            "model": stats.model,
            "prompt_cost": format(Decimal(repr(float(stats.prompt_cost))).normalize(), "f"),
            "completion_cost": format(Decimal(repr(float(stats.completion_cost))).normalize(), "f"),
        },
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "create":
        return {"calls": [create_outcome(call) for call in input_value["calls"]]}
    if input_value["operation"] == "job":
        return {"calls": [job_outcome(call) for call in input_value["calls"]]}
    if input_value["operation"] == "list":
        return {"calls": [list_outcome(call) for call in input_value["calls"]]}
    return {"calls": [row_outcome(call) for call in input_value["calls"]]}
