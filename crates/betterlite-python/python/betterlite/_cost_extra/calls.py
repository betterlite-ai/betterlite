import json
from typing import Final

from pydantic import JsonValue, TypeAdapter, ValidationError

from betterlite._cost_extra.wire import (
    ENGINE,
    FLOAT,
    BatchRequestWire,
    ImageRequestWire,
    OcrRequestWire,
    OcrUsageWire,
    RerankRequestWire,
    TranscriptionRequestWire,
    VectorStoreRequestWire,
    VideoRequestWire,
    native_answer,
    pair,
    usage_wire,
)
from betterlite.ocr import OCRResponse, OCRUsageInfo
from betterlite.types.rerank import RerankBilledUnits
from betterlite.types.utils.usage import Usage
from betterlite.types.vector_stores import VectorStoreSearchResponse

OCR_VALUE: Final = TypeAdapter(OCRResponse)


def ocr_usage(usage: OCRUsageInfo) -> OcrUsageWire:
    fields: OcrUsageWire = {
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
    # LiteLLM refuses anything but an OCRResponse, so None and every other type are rejected.
    refusal = f"response must be of type OCRResponse got type={type(response)}"
    try:
        typed = OCR_VALUE.validate_python(response, strict=True)
    except ValidationError as error:
        raise ValueError(refusal) from error
    if typed.usage_info is None:
        raise ValueError("OCR response usage_info is None")
    request: OcrRequestWire = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "usage": ocr_usage(typed.usage_info),
        "model_info": model_info,
    }
    return pair(ENGINE.ocr_cost, json.dumps(request))


def ocr_batch_cost(
    model: str,
    custom_llm_provider: str | None,
    usage_info: OCRUsageInfo,
    model_info: dict[str, JsonValue] | None = None,
) -> tuple[float, float]:
    usage = ocr_usage(usage_info)
    request: OcrRequestWire = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "usage": usage,
        "model_info": model_info,
    }
    return pair(ENGINE.ocr_batch_cost, json.dumps(request))


def vector_store_search_cost(
    model: str | None,
    custom_llm_provider: str | None,
    response: VectorStoreSearchResponse,
) -> tuple[float, float]:
    request: VectorStoreRequestWire = {"model": model, "custom_llm_provider": custom_llm_provider}
    request["response"] = response
    return pair(ENGINE.vector_store_cost, json.dumps(request))


def rerank_cost(
    model: str, custom_llm_provider: str | None, billed_units: RerankBilledUnits | None = None
) -> tuple[float, float]:
    request: RerankRequestWire = {"model": model, "custom_llm_provider": custom_llm_provider}
    if billed_units is not None:
        request["billed_units"] = billed_units
    return pair(ENGINE.rerank_cost, json.dumps(request))


def transcription_cost(
    model: str, custom_llm_provider: str | None, duration: float
) -> tuple[float, float]:
    request: TranscriptionRequestWire = {
        "model": model, "custom_llm_provider": custom_llm_provider, "duration": duration
    }
    return pair(ENGINE.transcription_cost, json.dumps(request))


def default_image_cost_calculator(  # noqa: PLR0913, PLR0917, ARG001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2407
    model: str,
    custom_llm_provider: str | None = None,
    quality: str | None = None,
    n: int | None = 1,
    size: str | None = "1024-x-1024",
    optional_params: dict[str, JsonValue] | None = None,
    model_info: dict[str, JsonValue] | None = None,
) -> float:
    request: ImageRequestWire = {
        "model": model,
        "custom_llm_provider": custom_llm_provider,
        "quality": quality,
        "n": n,
        "size": size,
        "model_info": model_info,
    }
    return native_answer(FLOAT, ENGINE.image_cost, json.dumps(request))


def default_video_cost_calculator(
    model: str,
    duration_seconds: float,
    custom_llm_provider: str | None = None,
    model_info: dict[str, JsonValue] | None = None,
    video_resolution: str | None = None,
) -> float:
    request: VideoRequestWire = {
        "model": model,
        "duration_seconds": duration_seconds,
        "custom_llm_provider": custom_llm_provider,
        "model_info": model_info,
    }
    if video_resolution is not None:
        request["video_resolution"] = video_resolution
    return native_answer(FLOAT, ENGINE.video_cost, json.dumps(request))


def batch_cost_calculator(
    usage: Usage,
    model: str,
    custom_llm_provider: str | None = None,
    model_info: dict[str, JsonValue] | None = None,
    data_residency: str | None = None,
) -> tuple[float, float]:
    counts = usage_wire(usage)
    request: BatchRequestWire = {"usage": counts, "model": model}
    request["custom_llm_provider"] = custom_llm_provider
    request["model_info"] = model_info
    request["data_residency"] = data_residency
    return pair(ENGINE.batch_cost, json.dumps(request))
