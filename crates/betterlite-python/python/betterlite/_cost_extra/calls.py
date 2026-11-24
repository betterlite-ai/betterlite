from pydantic import JsonValue

from betterlite._cost.wire import JSON_OBJECT
from betterlite._cost_extra.wire import (
    ENGINE,
    FLOAT,
    JsonObject,
    native_answer,
    pair,
    usage_wire,
)
from betterlite.ocr import OCRResponse, OCRUsageInfo
from betterlite.types.rerank import RerankBilledUnits
from betterlite.types.utils.usage import Usage
from betterlite.types.vector_stores import VectorStoreSearchResponse


def ocr_usage(usage: OCRUsageInfo) -> JsonObject:
    fields: JsonObject = {
        "credits": usage.credits,
        "pages_processed": usage.pages_processed,
        "pages_processed_annotation": usage.pages_processed_annotation,
    }
    return fields


def ocr_cost(
    model: str,
    custom_llm_provider: str | None,
    response: OCRResponse | None = None,
    model_info: dict[str, JsonValue] | None = None,
) -> tuple[float, float]:
    # LiteLLM accepts any object here and refuses the wrong type at runtime, so the check stays.
    if not isinstance(response, OCRResponse):  # ast-grep-ignore: no-isinstance
        raise ValueError(f"response must be of type OCRResponse got type={type(response)}")
    if response.usage_info is None:
        raise ValueError("OCR response usage_info is None")
    request: JsonObject = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "usage": ocr_usage(response.usage_info),
        "model_info": model_info,
    }
    return pair(ENGINE.ocr_cost, request)


def ocr_batch_cost(
    model: str,
    custom_llm_provider: str | None,
    usage_info: OCRUsageInfo,
    model_info: dict[str, JsonValue] | None = None,
) -> tuple[float, float]:
    usage = ocr_usage(usage_info)
    return pair(
        ENGINE.ocr_batch_cost,
        {
            "model": model,
            "custom_llm_provider": custom_llm_provider,
            "usage": usage,
            "model_info": model_info,
        },
    )


def vector_store_search_cost(
    model: str | None,
    custom_llm_provider: str | None,
    response: VectorStoreSearchResponse,
) -> tuple[float, float]:
    request: JsonObject = {"model": model, "custom_llm_provider": custom_llm_provider}
    request["response"] = JSON_OBJECT.validate_python(response)
    return pair(ENGINE.vector_store_cost, request)


def rerank_cost(
    model: str,
    custom_llm_provider: str | None,
    billed_units: RerankBilledUnits | None = None,
) -> tuple[float, float]:
    request: JsonObject = {}
    request["model"] = model
    request["custom_llm_provider"] = custom_llm_provider
    if billed_units is not None:
        request["billed_units"] = JSON_OBJECT.validate_python(billed_units)
    return pair(ENGINE.rerank_cost, request)


def transcription_cost(
    model: str, custom_llm_provider: str | None, duration: float
) -> tuple[float, float]:
    request: JsonObject = {"model": model, "custom_llm_provider": custom_llm_provider}
    request["duration"] = duration
    return pair(ENGINE.transcription_cost, request)


def default_image_cost_calculator(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2407
    model: str,
    custom_llm_provider: str | None = None,
    quality: str | None = None,
    n: int | None = 1,
    size: str | None = "1024-x-1024",
    optional_params: dict[str, JsonValue] | None = None,  # noqa: ARG001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2413
    model_info: dict[str, JsonValue] | None = None,
) -> float:
    request: JsonObject = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "quality": quality,
        "n": n,
        "size": size,
        "model_info": model_info,
    }
    return native_answer(FLOAT, ENGINE.image_cost, request)


def default_video_cost_calculator(
    model: str,
    duration_seconds: float,
    custom_llm_provider: str | None = None,
    model_info: dict[str, JsonValue] | None = None,
    video_resolution: str | None = None,
) -> float:
    request: JsonObject = {
        "model": model,
        "duration_seconds": duration_seconds,
        "custom_llm_provider": custom_llm_provider,
        "model_info": model_info,
    }
    if video_resolution is not None:
        request["video_resolution"] = video_resolution
    return native_answer(FLOAT, ENGINE.video_cost, request)


def batch_cost_calculator(
    usage: Usage,
    model: str,
    custom_llm_provider: str | None = None,
    model_info: dict[str, JsonValue] | None = None,
    data_residency: str | None = None,
) -> tuple[float, float]:
    counts = usage_wire(usage)
    request: JsonObject = {"usage": counts, "model": model}
    request["custom_llm_provider"] = custom_llm_provider
    request["model_info"] = model_info
    request["data_residency"] = data_residency
    return pair(ENGINE.batch_cost, request)
