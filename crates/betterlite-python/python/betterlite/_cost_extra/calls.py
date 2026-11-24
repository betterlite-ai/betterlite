import json
from collections.abc import Mapping

from pydantic import JsonValue

from betterlite._cost_extra.wire import (
    ENGINE,
    BatchRequestWire,
    ImageRequestWire,
    OcrBatchRequestWire,
    OcrRequestWire,
    OcrUsageWire,
    RerankRequestWire,
    TranscriptionRequestWire,
    VectorStoreRequestWire,
    VideoRequestWire,
    cost,
    pair,
    usage_wire,
)
from betterlite.ocr import OCRResponse, OCRUsageInfo
from betterlite.types.rerank import RerankBilledUnits
from betterlite.types.utils.usage import Usage
from betterlite.types.vector_stores import VectorStoreSearchResponse


def ocr_cost(
    model: str,
    custom_llm_provider: str | None,
    response: OCRResponse | None = None,
    model_info: Mapping[str, JsonValue] | None = None,
) -> tuple[float, float]:
    # LiteLLM accepts any object here and refuses the wrong type at runtime, so the check stays.
    if not isinstance(response, OCRResponse):  # ast-grep-ignore: no-isinstance
        raise ValueError(f"response must be of type OCRResponse got type={type(response)}")
    if response.usage_info is None:
        raise ValueError("OCR response usage_info is None")
    usage = response.usage_info
    request = OcrRequestWire(
        model=model,
        custom_llm_provider=custom_llm_provider,
        usage=OcrUsageWire(
            credits=usage.credits,
            pages_processed=usage.pages_processed,
            pages_processed_annotation=usage.pages_processed_annotation,
        ),
        model_info=model_info,
    )
    return pair(ENGINE.ocr_cost, json.dumps(request))


def ocr_batch_cost(
    model: str,
    custom_llm_provider: str | None,
    usage_info: OCRUsageInfo,
    model_info: Mapping[str, JsonValue] | None = None,
) -> tuple[float, float]:
    request = OcrBatchRequestWire(
        model=model,
        custom_llm_provider=custom_llm_provider,
        usage=OcrUsageWire(
            credits=usage_info.credits,
            pages_processed=usage_info.pages_processed,
            pages_processed_annotation=usage_info.pages_processed_annotation,
        ),
        model_info=model_info,
    )
    return pair(ENGINE.ocr_batch_cost, json.dumps(request))


def vector_store_search_cost(
    model: str | None,
    custom_llm_provider: str | None,
    response: VectorStoreSearchResponse,
) -> tuple[float, float]:
    request = VectorStoreRequestWire(
        model=model, custom_llm_provider=custom_llm_provider, response=response
    )
    return pair(ENGINE.vector_store_cost, json.dumps(request))


def rerank_cost(
    model: str,
    custom_llm_provider: str | None,
    billed_units: RerankBilledUnits | None = None,
) -> tuple[float, float]:
    request = RerankRequestWire(
        model=model, custom_llm_provider=custom_llm_provider, billed_units=billed_units
    )
    return pair(ENGINE.rerank_cost, json.dumps(request))


def transcription_cost(
    model: str, custom_llm_provider: str | None, duration: float
) -> tuple[float, float]:
    request = TranscriptionRequestWire(
        model=model, custom_llm_provider=custom_llm_provider, duration=duration
    )
    return pair(ENGINE.transcription_cost, json.dumps(request))


def default_image_cost_calculator(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2407
    model: str,
    custom_llm_provider: str | None = None,
    quality: str | None = None,
    n: int | None = 1,
    size: str | None = "1024-x-1024",
    optional_params: Mapping[str, JsonValue] | None = None,  # noqa: ARG001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2413
    model_info: Mapping[str, JsonValue] | None = None,
) -> float:
    request = ImageRequestWire(
        model=model,
        custom_llm_provider=custom_llm_provider,
        quality=quality,
        n=n,
        size=size,
        model_info=model_info,
    )
    return cost(ENGINE.image_cost, json.dumps(request))


def default_video_cost_calculator(
    model: str,
    duration_seconds: float,
    custom_llm_provider: str | None = None,
    model_info: Mapping[str, JsonValue] | None = None,
    video_resolution: str | None = None,
) -> float:
    request = VideoRequestWire(
        model=model,
        duration_seconds=duration_seconds,
        custom_llm_provider=custom_llm_provider,
        model_info=model_info,
        video_resolution=video_resolution,
    )
    return cost(ENGINE.video_cost, json.dumps(request))


def batch_cost_calculator(
    usage: Usage,
    model: str,
    custom_llm_provider: str | None = None,
    model_info: Mapping[str, JsonValue] | None = None,
    data_residency: str | None = None,
) -> tuple[float, float]:
    request = BatchRequestWire(
        usage=usage_wire(usage),
        model=model,
        custom_llm_provider=custom_llm_provider,
        model_info=model_info,
        data_residency=data_residency,
    )
    return pair(ENGINE.batch_cost, json.dumps(request))
