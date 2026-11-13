import os
from decimal import Decimal
from typing import Literal, TypeAlias, TypedDict, cast

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
from litellm.types.llms.vertex_ai import (
    GenerateContentResponseBody,
    VertexBatchPredictionResponse,
)
from litellm.types.utils import LiteLLMBatch, ModelInfo, Usage

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
MONEY_SCALE: str = "f"


class CreateCall(TypedDict):
    data: CreateBatchRequest
    vertex_project: str | None
    vertex_location: str | None


class JobCall(TypedDict):
    response: VertexBatchPredictionResponse


class ListCall(TypedDict):
    page: Object


class RowCall(TypedDict):
    row: Object
    model_name: str | None
    model_info: Object | None
    pricing: Literal["fixed", "requires_info", "failure"]


class CreateInput(TypedDict):
    operation: Literal["create"]
    calls: list[CreateCall]


class JobInput(TypedDict):
    operation: Literal["job"]
    zone: str
    calls: list[JobCall]


class ListInput(TypedDict):
    operation: Literal["list"]
    zone: str
    calls: list[ListCall]


class RowInput(TypedDict):
    operation: Literal["row"]
    calls: list[RowCall]


Input: TypeAlias = CreateInput | JobInput | ListInput | RowInput


class ListPage(TypedDict):
    object: str
    data: list[LiteLLMBatch]
    first_id: str | None
    last_id: str | None
    has_more: bool
    next_page_token: str | None


class PricingFailureError(Exception):
    pass


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
        if custom_llm_provider != "vertex_ai":
            raise PricingFailureError(custom_llm_provider)
        if self.pricing == "failure":
            raise PricingFailureError("pricing")
        if self.pricing == "requires_info" and model_info is None:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
            raise PricingFailureError("model_info")
        return (0.25, 0.5)


def calculated_usage(body: GenerateContentResponseBody) -> Usage:
    return VertexGeminiConfig._calculate_usage(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1742
        body,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L471
    )


def money(value: float) -> str:
    exact = Decimal(repr(float(value))).normalize()
    return format(exact, MONEY_SCALE)


def local_zone(zone: str) -> None:
    if os.environ.get("TZ") != zone:
        raise ValueError(f"the case TZ must match the zone {zone}")


def job_state(batch: LiteLLMBatch) -> Json:
    state: Json = {
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
    return state


def usage_state(usage: Usage) -> Json:
    prompt = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    prompt_state: Json = None
    if prompt is not None:
        prompt_state = {
            "text_tokens": prompt.text_tokens,
            "audio_tokens": prompt.audio_tokens,
            "image_tokens": prompt.image_tokens,
            "video_tokens": prompt.video_tokens,
            "cached_tokens": prompt.cached_tokens,
            "tool_use_tokens": prompt.tool_use_tokens,
            "web_search_requests": prompt.web_search_requests,
            "google_maps_grounding_requests": prompt.google_maps_grounding_requests,
        }
    completion_state: Json = None
    if completion is not None:
        completion_state = {
            "text_tokens": completion.text_tokens,
            "audio_tokens": completion.audio_tokens,
            "image_tokens": completion.image_tokens,
            "video_tokens": completion.video_tokens,
            "reasoning_tokens": completion.reasoning_tokens,
        }
    state: Json = {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_tokens_details": prompt_state,
        "completion_tokens_details": completion_state,
    }
    return state


def create_outcome(call: CreateCall) -> Json:
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
    return {
        "ok": True,
        "job": cast(Json, job),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L694
    }


def job_outcome(call: JobCall) -> Json:
    status: Json = None
    try:
        batch = BatchTransform.transform_vertex_ai_batch_response_to_openai_batch_response(
            call["response"],
        )
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        status = error.status_code
    except (AttributeError, KeyError, TypeError, ValueError):
        status = None
    else:
        return {"ok": True, "batch": job_state(batch)}
    return {"ok": False, "status": status}


def list_outcome(call: ListCall) -> Json:
    try:
        listed = BatchTransform.transform_vertex_ai_batch_list_response_to_openai_list_response(
            call["page"],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
        )
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        return {"ok": False, "status": error.status_code}
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    page = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
        ListPage, listed
    )
    state: Json = {
        "object": page["object"],
        "data": [job_state(batch) for batch in page["data"]],
        "first_id": page["first_id"],
        "last_id": page["last_id"],
        "has_more": page["has_more"],
        "next_page_token": page["next_page_token"],
    }
    return {"ok": True, "page": state}


def row_outcome(call: RowCall) -> Json:
    stats = native_vertex_batch_row_stats(
        call["row"],
        call["model_name"],
        model_info=cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
            "ModelInfo | None", call["model_info"]
        ),
        calculate_usage=calculated_usage,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L471
        cost_calculator=FixturePricing(call["pricing"]),
    )
    if stats is None:
        return {"ok": True, "stats": None}
    state: Json = {
        "usage": usage_state(stats.usage),
        "total_tokens": stats.total_tokens,
        "model": stats.model,
        "prompt_cost": money(stats.prompt_cost),
        "completion_cost": money(stats.completion_cost),
    }
    return {"ok": True, "stats": state}


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case "create":
            return {"calls": [create_outcome(call) for call in input_value["calls"]]}
        case "job":
            local_zone(input_value["zone"])
            return {"calls": [job_outcome(call) for call in input_value["calls"]]}
        case "list":
            local_zone(input_value["zone"])
            return {"calls": [list_outcome(call) for call in input_value["calls"]]}
        case "row":
            return {"calls": [row_outcome(call) for call in input_value["calls"]]}
