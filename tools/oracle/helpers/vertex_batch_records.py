from decimal import Decimal
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

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


class CreateCall(TypedDict):
    data: CreateBatchRequest
    vertex_project: str | None
    vertex_location: str | None


class RowCall(TypedDict):
    row: Object
    model_name: str | None
    model_info: Object | None
    pricing: Literal["fixed", "requires_info", "failure"]


class Input(TypedDict):
    operation: Literal["create", "job", "list", "row"]
    zone: NotRequired[str]
    calls: list[Json]


class ListPage(TypedDict):
    object: str
    data: list[LiteLLMBatch]
    first_id: str | None
    last_id: str | None
    has_more: bool
    next_page_token: str | None


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


def calculated_usage(body: GenerateContentResponseBody) -> Usage:
    return VertexGeminiConfig._calculate_usage(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1742
        body,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L471
    )


def job_state(batch: LiteLLMBatch) -> Json:
    state: Object = {}
    state["id"] = batch.id
    state["completion_window"] = batch.completion_window
    state["created_at"] = batch.created_at
    state["endpoint"] = batch.endpoint
    state["input_file_id"] = batch.input_file_id
    state["object"] = batch.object
    state["status"] = batch.status
    state["error_file_id"] = batch.error_file_id
    state["output_file_id"] = batch.output_file_id
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
    return {"ok": True, "job": cast(Json, job)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L694


def job_outcome(response: VertexBatchPredictionResponse) -> Json:
    status: Json = None
    try:
        batch = BatchTransform.transform_vertex_ai_batch_response_to_openai_batch_response(
            response,
        )
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        status = error.status_code
    except (AttributeError, KeyError, TypeError, ValueError):
        status = None
    else:
        return {"ok": True, "batch": job_state(batch)}
    return {"ok": False, "status": status}


def list_outcome(response: Object) -> Json:
    try:
        listed = BatchTransform.transform_vertex_ai_batch_list_response_to_openai_list_response(
            response,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
        )
    except VertexAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/common_utils.py#L99
        return {"ok": False, "status": error.status_code}
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    page = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
        ListPage, listed
    )
    state: Object = {}
    state["object"] = page["object"]
    state["data"] = [job_state(batch) for batch in page["data"]]
    state["first_id"] = page["first_id"]
    state["last_id"] = page["last_id"]
    state["has_more"] = page["has_more"]
    state["next_page_token"] = page["next_page_token"]
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
        "usage": {
            "prompt_tokens": stats.usage.prompt_tokens,
            "completion_tokens": stats.usage.completion_tokens,
            "total_tokens": stats.usage.total_tokens,
        },
        "total_tokens": stats.total_tokens,
        "model": stats.model,
        "prompt_cost": format(Decimal(repr(float(stats.prompt_cost))).normalize(), "f"),
        "completion_cost": format(Decimal(repr(float(stats.completion_cost))).normalize(), "f"),
    }
    return {"ok": True, "stats": state}


def run(input_value: Input) -> Json:
    calls = input_value["calls"]
    match input_value["operation"]:
        case "create":
            create_calls = cast("list[CreateCall]", calls)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L517
            return {"calls": [create_outcome(call) for call in create_calls]}
        case "job":
            job_calls = cast("list[VertexBatchPredictionResponse]", calls)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L694
            return {"calls": [job_outcome(call) for call in job_calls]}
        case "list":
            list_calls = cast("list[Object]", calls)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/transformation.py#L196
            return {"calls": [list_outcome(page) for page in list_calls]}
        case "row":
            row_calls = cast("list[RowCall]", calls)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/vertex_ai.py#L694
            return {"calls": [row_outcome(call) for call in row_calls]}
