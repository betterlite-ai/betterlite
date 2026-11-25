import importlib
import inspect
from collections.abc import Mapping, Sequence
from inspect import Signature
from typing import Final, NotRequired, Protocol, TypeAlias, cast

import pytest
from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite import completion_cost
from betterlite._cost_extra import (
    BaseTokenUsageProcessor,
    RealtimeAPITokenUsageProcessor,
    batch_cost_calculator,
    default_image_cost_calculator,
    default_video_cost_calculator,
    ocr_batch_cost,
    ocr_cost,
    rerank_cost,
    transcription_cost,
    vector_store_search_cost,
)
from betterlite._cost_extra import (
    ResponsesWebSocketTokenUsageProcessor as WsUsageProcessor,
)
from betterlite._cost_extra import (
    handle_realtime_stream_cost_calculation as realtime_stream_cost,
)
from betterlite._cost_extra import (
    handle_realtime_transcription_cost_calculation as realtime_transcription_cost,
)
from betterlite.cost_calculator import (
    batch_cost_calculator as module_batch_cost_calculator,
)
from betterlite.cost_calculator import (
    completion_cost as module_completion_cost,
)
from betterlite.cost_calculator import (
    ocr_cost as module_ocr_cost,
)
from betterlite.ocr import OCRPage, OCRResponse, OCRUsageInfo
from betterlite.types.rerank import RerankBilledUnits
from betterlite.types.utils import Usage
from betterlite.types.vector_stores import VectorStoreSearchResponse

OCR_MODEL: Final = "mistral/mistral-ocr-latest"
OCR_PAGES: Final = 3
OCR_BATCH_PAGES: Final = 4
OCR_ANNOTATION_PAGES: Final = 1
VECTOR_MODEL: Final = "vertex_ai/search_api"
RERANK_MODEL: Final = "rerank-v3.5"
COHERE: Final = "cohere"
RERANK_UNITS_COUNT: Final = 2
TRANSCRIPTION_MODEL: Final = "whisper-1"
TRANSCRIPTION_SECONDS: Final = 2.5
IMAGE_MODEL: Final = "gpt-image-1"
SIZED_IMAGE_MODEL: Final = "gpt-image-1.5"
DALL_E_MODEL: Final = "dall-e-3"
DALL_E_PRICE: Final = 0.04
DALL_E_IMAGES: Final = 2
IMAGE_QUALITY: Final = "high"
IMAGE_SIZE: Final = "1024x1024"
IMAGE_WIDE_SIZE: Final = "1024x1536"
VIDEO_MODEL: Final = "veo-3.1-generate-001"
VERTEX: Final = "vertex_ai"
VIDEO_SECONDS: Final = 8.0
VIDEO_RESOLUTION: Final = "720p"
BATCH_MODEL: Final = "gpt-5.5"
OPENAI: Final = "openai"
EU_RESIDENCY: Final = "eu"
BATCH_PROMPT_TOKENS: Final = 1000
BATCH_COMPLETION_TOKENS: Final = 500
CACHED_TOKENS_PER_CALL: Final = 50
MIXED_ALIAS_TOKENS: Final = 10
MIXED_DETAIL_TOKENS: Final = 20
SERVER_TOOL_REQUESTS: Final = 2
UNKNOWN_PROVIDER: Final = "typo-provider"
REALTIME_MODEL: Final = "gpt-realtime"
REALTIME_PROMPT_TOKENS: Final = 100
REALTIME_COMPLETION_TOKENS: Final = 20
REALTIME_CACHED_TOKENS: Final = 50
REALTIME_AUDIO_TOKENS: Final = 50
REALTIME_OUTPUT_AUDIO_TOKENS: Final = 10
CACHED_AUDIO_TOKENS: Final = 10
WS_FIRST_PROMPT: Final = 10
WS_FIRST_COMPLETION: Final = 2
WS_SECOND_PROMPT: Final = 5
WS_SECOND_COMPLETION: Final = 1
WS_THIRD_PROMPT: Final = 3
WS_THIRD_COMPLETION: Final = 1
RELATIVE: Final = 1e-12
ZERO: Final = 0.0

PAIR: Final[TypeAdapter[tuple[float, float]]] = TypeAdapter(tuple[float, float])
FLOAT: Final[TypeAdapter[float]] = TypeAdapter(float)


class CountsView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cached_tokens: int
    cached_audio_tokens: int
    prompt_audio_tokens: int
    completion_audio_tokens: int


class CachedDetailsView(TypedDict, total=False):
    audio_tokens: int


class DetailsView(TypedDict, total=False):
    cached_tokens: int
    audio_tokens: int
    text_tokens: int
    cached_tokens_details: NotRequired[CachedDetailsView | None]


class ImageArgs(TypedDict):
    model: str
    quality: str | None
    n: int | None
    size: str | None
    model_info: dict[str, JsonValue] | None


class DumpView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: NotRequired[DetailsView | None]
    completion_tokens_details: NotRequired[DetailsView | None]


DUMP: Final[TypeAdapter[DumpView]] = TypeAdapter(DumpView)


# LiteLLM's namespace modules carry no stubs, so its calls come back as JSON and are validated here.
class ReferenceUsage(Protocol):
    def model_dump_json(self) -> str: ...


ReferenceValue: TypeAlias = JsonValue | ReferenceUsage | list[dict[str, JsonValue]]


class ReferenceCall(Protocol):
    def __call__(self, *args: ReferenceValue, **kwargs: ReferenceValue) -> JsonValue: ...


class ReferenceModule(Protocol):
    def __getattr__(self, name: str) -> ReferenceCall: ...


class ReferenceProcessor(Protocol):
    def combine_usage_objects(
        self, usage_objects: list[ReferenceUsage]
    ) -> ReferenceUsage: ...

    def collect_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        self, results: list[dict[str, JsonValue]]
    ) -> list[ReferenceUsage]: ...

    def collect_and_combine_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        self, results: list[dict[str, JsonValue]]
    ) -> ReferenceUsage: ...

    def collect_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        self, results: list[dict[str, JsonValue]]
    ) -> tuple[ReferenceUsage, ...]: ...

    def partition_results_by_service_tier(  # ast-grep-ignore: no-long-names
        self, results: list[dict[str, JsonValue]]
    ) -> Mapping[str | None, tuple[dict[str, JsonValue], ...]]: ...

    def collect_and_combine_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        self, results: list[dict[str, JsonValue]]
    ) -> ReferenceUsage: ...


class ReferenceUsageFactory(Protocol):
    def __call__(self, **values: JsonValue) -> ReferenceUsage: ...


class ReferenceProcessors(Protocol):
    Usage: ReferenceUsageFactory
    BaseTokenUsageProcessor: ReferenceProcessor
    RealtimeAPITokenUsageProcessor: ReferenceProcessor
    ResponsesWebSocketTokenUsageProcessor: ReferenceProcessor  # ast-grep-ignore: no-long-names


REFERENCE: ReferenceModule = importlib.import_module("litellm.cost_calculator")
REFERENCE_OCR: ReferenceModule = importlib.import_module(
    "litellm.llms.base_llm.ocr.transformation"
)
REFERENCE_PROCESSORS = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L2764-L2884
    ReferenceProcessors, importlib.import_module("litellm.cost_calculator")
)

RERANK_UNITS: Final[RerankBilledUnits] = {"search_units": RERANK_UNITS_COUNT}
REFERENCE_RERANK_UNITS: Final[dict[str, JsonValue]] = {"search_units": RERANK_UNITS_COUNT}
VECTOR_RESPONSE: Final[VectorStoreSearchResponse] = {
    "object": "vector_store.search_results.page",
    "search_query": "query",
    "data": [],
}
REFERENCE_VECTOR_RESPONSE: Final[dict[str, JsonValue]] = {
    "object": "vector_store.search_results.page",
    "search_query": "query",
    "data": [],
}
DALL_E_INFO: Final[dict[str, JsonValue]] = {"input_cost_per_image": DALL_E_PRICE}
DALL_E_ARGS: Final[ImageArgs] = {
    "model": DALL_E_MODEL,
    "quality": "standard",
    "n": DALL_E_IMAGES,
    "size": IMAGE_SIZE,
    "model_info": DALL_E_INFO,
}
PAGES: Final[list[OCRPage]] = []
REFERENCE_PAGES: Final[list[JsonValue]] = []
REALTIME_USAGE: Final[dict[str, JsonValue]] = {
    "type": "response.done",
    "response": {
        "usage": {
            "input_tokens": REALTIME_PROMPT_TOKENS,
            "output_tokens": REALTIME_COMPLETION_TOKENS,
            "total_tokens": REALTIME_PROMPT_TOKENS + REALTIME_COMPLETION_TOKENS,
            "input_token_details": {
                "cached_tokens": REALTIME_CACHED_TOKENS,
                "audio_tokens": REALTIME_AUDIO_TOKENS,
                "text_tokens": REALTIME_AUDIO_TOKENS,
            },
            "output_token_details": {
                "audio_tokens": REALTIME_OUTPUT_AUDIO_TOKENS,
                "text_tokens": REALTIME_OUTPUT_AUDIO_TOKENS,
            },
        }
    },
}
REALTIME_EVENTS: Final[list[dict[str, JsonValue]]] = [
    {"type": "session.created", "session": {"model": REALTIME_MODEL}},
    REALTIME_USAGE,
    {"type": "response.done", "response": {}},
]
WS_EVENTS: Final[list[dict[str, JsonValue]]] = [
    {
        "type": "response.completed",
        "response": {
            "service_tier": "default",
            "usage": {
                "input_tokens": WS_FIRST_PROMPT,
                "output_tokens": WS_FIRST_COMPLETION,
                "total_tokens": WS_FIRST_PROMPT + WS_FIRST_COMPLETION,
            },
        },
    },
    {
        "type": "response.completed",
        "response": {
            "service_tier": "flex",
            "usage": {
                "input_tokens": WS_SECOND_PROMPT,
                "output_tokens": WS_SECOND_COMPLETION,
                "total_tokens": WS_SECOND_PROMPT + WS_SECOND_COMPLETION,
            },
        },
    },
    {
        "type": "response.incomplete",
        "response": {
            "usage": {
                "input_tokens": WS_THIRD_PROMPT,
                "output_tokens": WS_THIRD_COMPLETION,
                "total_tokens": WS_THIRD_PROMPT + WS_THIRD_COMPLETION,
            },
        },
    },
    {
        "type": "response.created",
        "response": {
            "usage": {
                "input_tokens": REALTIME_PROMPT_TOKENS,
                "output_tokens": REALTIME_COMPLETION_TOKENS,
                "total_tokens": REALTIME_PROMPT_TOKENS + REALTIME_COMPLETION_TOKENS,
            },
        },
    },
    {"type": "response.completed", "response": {"service_tier": "default"}},
]
TRANSCRIPTION_EVENTS: Final[list[dict[str, JsonValue]]] = [
    {
        "type": "conversation.item.input_audio_transcription.completed",
        "usage": {"type": "duration", "seconds": TRANSCRIPTION_SECONDS},
    }
]


class ParameterView(Protocol):
    @property
    def default(self) -> JsonValue: ...


def signature_rows(signature: Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        view: ParameterView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(view.default)))
    return rows


def assert_pair_matches(ours: tuple[float, float], reference: tuple[float, float]) -> None:
    assert ours[0] == pytest.approx(reference[0], rel=RELATIVE)
    assert ours[1] == pytest.approx(reference[1], rel=RELATIVE)


def our_ocr(pages: int) -> OCRResponse:
    return OCRResponse(
        pages=PAGES, model=OCR_MODEL, usage_info=OCRUsageInfo(pages_processed=pages)
    )


def reference_ocr(pages: int) -> JsonValue:
    return REFERENCE_OCR.OCRResponse(
        pages=REFERENCE_PAGES,
        model=OCR_MODEL,
        usage_info=REFERENCE_OCR.OCRUsageInfo(pages_processed=pages),
    )


def our_usage(prompt: int, completion: int, cached: int) -> Usage:
    return Usage(
        prompt_tokens=prompt,
        completion_tokens=completion,
        total_tokens=prompt + completion,
        cache_read_input_tokens=cached,
    )


def reference_usage(
    prompt: int,
    completion: int,
    cached: int | None,
    prompt_details: dict[str, JsonValue] | None = None,
) -> ReferenceUsage:
    values: dict[str, JsonValue] = {
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    }
    if cached is not None:
        values["cache_read_input_tokens"] = cached
    if prompt_details is not None:
        values["prompt_tokens_details"] = prompt_details
    return REFERENCE_PROCESSORS.Usage(**values)


def usage_counts(usage: Usage | ReferenceUsage) -> CountsView:
    dump = DUMP.validate_json(usage.model_dump_json())
    prompt = dump.get("prompt_tokens_details")
    completion = dump.get("completion_tokens_details")
    cached = prompt["cached_tokens"] if prompt is not None and "cached_tokens" in prompt else 0
    cached_details = prompt.get("cached_tokens_details") if prompt is not None else None
    cached_audio = (
        cached_details["audio_tokens"]
        if cached_details is not None and "audio_tokens" in cached_details
        else 0
    )
    prompt_audio = prompt["audio_tokens"] if prompt is not None and "audio_tokens" in prompt else 0
    completion_audio = (
        completion["audio_tokens"]
        if completion is not None and "audio_tokens" in completion
        else 0
    )
    return CountsView(
        prompt_tokens=dump["prompt_tokens"],
        completion_tokens=dump["completion_tokens"],
        total_tokens=dump["total_tokens"],
        cached_tokens=cached,
        cached_audio_tokens=cached_audio,
        prompt_audio_tokens=prompt_audio,
        completion_audio_tokens=completion_audio,
    )


def compare_pair(ours: tuple[float, float], reference: JsonValue) -> None:
    expected = PAIR.validate_python(reference)
    assert_pair_matches(ours, expected)


def compare_float(ours: float, reference: JsonValue) -> None:
    expected = FLOAT.validate_python(reference)
    assert ours == pytest.approx(expected, rel=RELATIVE)


def compare_usages(ours: Sequence[Usage], reference: Sequence[ReferenceUsage]) -> None:
    assert [usage_counts(usage) for usage in ours] == [
        usage_counts(usage) for usage in reference
    ]


def assert_realtime_counts(usage: Usage) -> None:
    counts = usage_counts(usage)
    assert counts["cached_tokens"] == REALTIME_CACHED_TOKENS
    assert counts["prompt_audio_tokens"] == REALTIME_AUDIO_TOKENS


BATCH_USAGE: Final[OCRUsageInfo] = OCRUsageInfo(
    pages_processed=OCR_BATCH_PAGES, pages_processed_annotation=OCR_ANNOTATION_PAGES
)
REFERENCE_BATCH_USAGE: Final = REFERENCE_OCR.OCRUsageInfo(
    pages_processed=OCR_BATCH_PAGES, pages_processed_annotation=OCR_ANNOTATION_PAGES
)
COMBINED_USAGES: Final[list[Usage]] = [
    our_usage(REALTIME_PROMPT_TOKENS, REALTIME_COMPLETION_TOKENS, CACHED_TOKENS_PER_CALL),
    our_usage(REALTIME_COMPLETION_TOKENS, REALTIME_COMPLETION_TOKENS, CACHED_TOKENS_PER_CALL),
]
REFERENCE_COMBINED_USAGES: Final[list[ReferenceUsage]] = [
    reference_usage(REALTIME_PROMPT_TOKENS, REALTIME_COMPLETION_TOKENS, CACHED_TOKENS_PER_CALL),
    reference_usage(
        REALTIME_COMPLETION_TOKENS, REALTIME_COMPLETION_TOKENS, CACHED_TOKENS_PER_CALL
    ),
]
REFERENCE_COMBINED_USAGE: Final = (
    REFERENCE_PROCESSORS.BaseTokenUsageProcessor.combine_usage_objects(
        REFERENCE_COMBINED_USAGES
    )
)
MIXED_DETAILS: Final[dict[str, JsonValue]] = {"cached_tokens": MIXED_DETAIL_TOKENS}
MIXED_USAGES: Final[list[Usage]] = [
    Usage(
        prompt_tokens=MIXED_DETAIL_TOKENS,
        completion_tokens=1,
        total_tokens=MIXED_DETAIL_TOKENS + 1,
        prompt_tokens_details=MIXED_DETAILS,
    ),
    Usage(
        prompt_tokens=MIXED_ALIAS_TOKENS,
        completion_tokens=1,
        total_tokens=MIXED_ALIAS_TOKENS + 1,
        cache_read_input_tokens=MIXED_ALIAS_TOKENS,
    ),
]
REFERENCE_MIXED_USAGES: Final[list[ReferenceUsage]] = [
    reference_usage(MIXED_DETAIL_TOKENS, 1, None, MIXED_DETAILS),
    reference_usage(MIXED_ALIAS_TOKENS, 1, MIXED_ALIAS_TOKENS),
]
SERVER_TOOL_USAGE: Final[dict[str, JsonValue]] = {
    "input_tokens": WS_FIRST_PROMPT,
    "output_tokens": WS_FIRST_COMPLETION,
    "total_tokens": WS_FIRST_PROMPT + WS_FIRST_COMPLETION,
    "server_tool_use": {"web_search_requests": SERVER_TOOL_REQUESTS},
}
SERVER_TOOL_REALTIME_EVENT: Final[dict[str, JsonValue]] = {
    "type": "response.done",
    "response": {"usage": SERVER_TOOL_USAGE},
}
SERVER_TOOL_WS_EVENT: Final[dict[str, JsonValue]] = {
    "type": "response.completed",
    "response": {"usage": SERVER_TOOL_USAGE},
}
COMBINED_REALTIME_USAGE: Final = (
    RealtimeAPITokenUsageProcessor.collect_and_combine_usage_from_realtime_stream_results(
        REALTIME_EVENTS
    )
)
REFERENCE_REALTIME_USAGES: Final = (
    REFERENCE_PROCESSORS.RealtimeAPITokenUsageProcessor
    .collect_usage_from_realtime_stream_results([REALTIME_USAGE])
)
REFERENCE_REALTIME_USAGE: Final = (
    REFERENCE_PROCESSORS.RealtimeAPITokenUsageProcessor
    .collect_and_combine_usage_from_realtime_stream_results(REALTIME_EVENTS)
)
REFERENCE_WS_USAGES: Final = (
    REFERENCE_PROCESSORS.ResponsesWebSocketTokenUsageProcessor
    .collect_usage_from_responses_ws_results(WS_EVENTS)
)
REFERENCE_WS_COMBINED: Final = (
    REFERENCE_PROCESSORS.ResponsesWebSocketTokenUsageProcessor
    .collect_and_combine_usage_from_responses_ws_results(WS_EVENTS)
)
REFERENCE_WS_PARTITION: Final = (
    REFERENCE_PROCESSORS.ResponsesWebSocketTokenUsageProcessor
    .partition_results_by_service_tier(WS_EVENTS)
)
REFERENCE_OCR_PAIR: Final = REFERENCE.ocr_cost(
    model=OCR_MODEL, custom_llm_provider=None, response=reference_ocr(OCR_PAGES)
)
REFERENCE_OCR_BATCH_PAIR: Final = REFERENCE.ocr_batch_cost(
    model=OCR_MODEL, custom_llm_provider=None, usage_info=REFERENCE_BATCH_USAGE
)
REFERENCE_VECTOR_PAIR: Final = REFERENCE.vector_store_search_cost(
    model=VECTOR_MODEL, custom_llm_provider=VERTEX, response=REFERENCE_VECTOR_RESPONSE
)
REFERENCE_RERANK_PAIR: Final = REFERENCE.rerank_cost(
    model=RERANK_MODEL, custom_llm_provider=COHERE, billed_units=REFERENCE_RERANK_UNITS
)
REFERENCE_TRANSCRIPTION_PAIR: Final = REFERENCE.transcription_cost(
    model=TRANSCRIPTION_MODEL, custom_llm_provider=None, duration=TRANSCRIPTION_SECONDS
)
REFERENCE_IMAGE_DEFAULT: Final = REFERENCE.default_image_cost_calculator(model=IMAGE_MODEL)
REFERENCE_IMAGE_SIZED: Final = REFERENCE.default_image_cost_calculator(
    model=SIZED_IMAGE_MODEL, quality=IMAGE_QUALITY, size=IMAGE_WIDE_SIZE
)
REFERENCE_IMAGE_INFO: Final = REFERENCE.default_image_cost_calculator(**DALL_E_ARGS)
REFERENCE_VIDEO_DURATION: Final = REFERENCE.default_video_cost_calculator(
    model=VIDEO_MODEL, duration_seconds=VIDEO_SECONDS, custom_llm_provider=VERTEX
)
REFERENCE_VIDEO_RESOLUTION: Final = REFERENCE.default_video_cost_calculator(
    model=VIDEO_MODEL,
    duration_seconds=VIDEO_SECONDS,
    custom_llm_provider=VERTEX,
    video_resolution=VIDEO_RESOLUTION,
)
REFERENCE_BATCH_PAIR: Final = REFERENCE.batch_cost_calculator(
    usage=reference_usage(BATCH_PROMPT_TOKENS, BATCH_COMPLETION_TOKENS, 0),
    model=BATCH_MODEL,
    custom_llm_provider=OPENAI,
    data_residency=EU_RESIDENCY,
)
REFERENCE_REALTIME_COST: Final = REFERENCE.handle_realtime_stream_cost_calculation(
    results=REALTIME_EVENTS,
    combined_usage_object=REFERENCE_REALTIME_USAGE,
    custom_llm_provider=OPENAI,
    litellm_model_name=REALTIME_MODEL,
)
REFERENCE_TRANSCRIPTION_COST: Final = REFERENCE.handle_realtime_transcription_cost_calculation(
    results=TRANSCRIPTION_EVENTS,
    custom_llm_provider=OPENAI,
    litellm_model_name=TRANSCRIPTION_MODEL,
)


def test_cost_extra_signatures() -> None:
    assert signature_rows(inspect.signature(ocr_cost)) == signature_rows(
        inspect.signature(REFERENCE.ocr_cost)
    )
    assert signature_rows(inspect.signature(ocr_batch_cost)) == signature_rows(
        inspect.signature(REFERENCE.ocr_batch_cost)
    )
    assert signature_rows(inspect.signature(vector_store_search_cost)) == signature_rows(
        inspect.signature(REFERENCE.vector_store_search_cost)
    )
    assert signature_rows(inspect.signature(rerank_cost)) == signature_rows(
        inspect.signature(REFERENCE.rerank_cost)
    )
    assert signature_rows(inspect.signature(transcription_cost)) == signature_rows(
        inspect.signature(REFERENCE.transcription_cost)
    )
    assert signature_rows(inspect.signature(default_image_cost_calculator)) == signature_rows(
        inspect.signature(REFERENCE.default_image_cost_calculator)
    )
    assert signature_rows(inspect.signature(default_video_cost_calculator)) == signature_rows(
        inspect.signature(REFERENCE.default_video_cost_calculator)
    )
    assert signature_rows(inspect.signature(batch_cost_calculator)) == signature_rows(
        inspect.signature(REFERENCE.batch_cost_calculator)
    )
    assert signature_rows(inspect.signature(realtime_stream_cost)) == signature_rows(
        inspect.signature(REFERENCE.handle_realtime_stream_cost_calculation)
    )
    assert signature_rows(
        inspect.signature(realtime_transcription_cost)
    ) == signature_rows(
        inspect.signature(REFERENCE.handle_realtime_transcription_cost_calculation)
    )


def test_ocr_cost_pages() -> None:
    ours = ocr_cost(OCR_MODEL, None, our_ocr(OCR_PAGES))
    compare_pair(ours, REFERENCE_OCR_PAIR)
    assert ours[0] > ZERO


def test_ocr_batch_cost_pages() -> None:
    compare_pair(
        ocr_batch_cost(OCR_MODEL, None, BATCH_USAGE), REFERENCE_OCR_BATCH_PAIR
    )


def test_vector_store_cost_vertex() -> None:
    ours = vector_store_search_cost(VECTOR_MODEL, VERTEX, VECTOR_RESPONSE)
    compare_pair(ours, REFERENCE_VECTOR_PAIR)
    assert ours[0] > ZERO


def test_vector_unknown_provider() -> None:
    with pytest.raises(ValueError, match="is not a valid LlmProviders"):
        vector_store_search_cost(None, UNKNOWN_PROVIDER, VECTOR_RESPONSE)
    with pytest.raises(ValueError, match="is not a valid LlmProviders"):
        REFERENCE.vector_store_search_cost(
            model=None,
            custom_llm_provider=UNKNOWN_PROVIDER,
            response=REFERENCE_VECTOR_RESPONSE,
        )


def test_rerank_cost_units() -> None:
    ours = rerank_cost(RERANK_MODEL, COHERE, RERANK_UNITS)
    compare_pair(ours, REFERENCE_RERANK_PAIR)
    assert ours[0] > ZERO


def test_transcription_duration() -> None:
    ours = transcription_cost(TRANSCRIPTION_MODEL, None, TRANSCRIPTION_SECONDS)
    compare_pair(ours, REFERENCE_TRANSCRIPTION_PAIR)
    assert ours[1] > ZERO


def test_image_cost_defaults() -> None:
    ours = default_image_cost_calculator(model=IMAGE_MODEL)
    compare_float(ours, REFERENCE_IMAGE_DEFAULT)


def test_image_cost_quality_size() -> None:
    ours = default_image_cost_calculator(
        model=SIZED_IMAGE_MODEL, quality=IMAGE_QUALITY, size=IMAGE_WIDE_SIZE
    )
    compare_float(ours, REFERENCE_IMAGE_SIZED)


def test_image_cost_model_info() -> None:
    ours = default_image_cost_calculator(**DALL_E_ARGS)
    compare_float(ours, REFERENCE_IMAGE_INFO)


def test_video_cost_duration() -> None:
    ours = default_video_cost_calculator(
        model=VIDEO_MODEL, duration_seconds=VIDEO_SECONDS, custom_llm_provider=VERTEX
    )
    compare_float(ours, REFERENCE_VIDEO_DURATION)
    assert ours > ZERO


def test_video_cost_resolution() -> None:
    ours = default_video_cost_calculator(
        model=VIDEO_MODEL,
        duration_seconds=VIDEO_SECONDS,
        custom_llm_provider=VERTEX,
        video_resolution=VIDEO_RESOLUTION,
    )
    compare_float(ours, REFERENCE_VIDEO_RESOLUTION)


def test_batch_cost_eu_residency() -> None:
    ours = batch_cost_calculator(
        our_usage(BATCH_PROMPT_TOKENS, BATCH_COMPLETION_TOKENS, 0),
        BATCH_MODEL,
        custom_llm_provider=OPENAI,
        data_residency=EU_RESIDENCY,
    )
    compare_pair(ours, REFERENCE_BATCH_PAIR)
    assert ours[0] > ZERO


def test_processor_combines() -> None:
    ours = BaseTokenUsageProcessor.combine_usage_objects(COMBINED_USAGES)
    compare_usages([ours], [REFERENCE_COMBINED_USAGE])
    assert usage_counts(ours)["prompt_tokens"] == (
        REALTIME_PROMPT_TOKENS + REALTIME_COMPLETION_TOKENS
    )


def test_processor_mixed_cache() -> None:
    ours = BaseTokenUsageProcessor.combine_usage_objects(MIXED_USAGES)
    reference = REFERENCE_PROCESSORS.BaseTokenUsageProcessor.combine_usage_objects(
        REFERENCE_MIXED_USAGES
    )
    compare_usages([ours], [reference])
    expected_cached_tokens = MIXED_ALIAS_TOKENS + MIXED_DETAIL_TOKENS
    assert usage_counts(ours)["cached_tokens"] == expected_cached_tokens
    assert usage_counts(reference)["cached_tokens"] == expected_cached_tokens


def test_processor_cached_audio() -> None:
    usage: dict[str, JsonValue] = {
        "input_tokens": 30,
        "output_tokens": 0,
        "total_tokens": 30,
        "input_tokens_details": {
            "cached_tokens": 20,
            "cached_tokens_details": {"audio_tokens": CACHED_AUDIO_TOKENS},
        },
    }
    realtime: dict[str, JsonValue] = {"type": "response.done", "response": {"usage": usage}}
    realtime_ours = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(
        [realtime]
    )
    realtime_reference = (
        REFERENCE_PROCESSORS.RealtimeAPITokenUsageProcessor
        .collect_usage_from_realtime_stream_results([realtime])
    )
    compare_usages(realtime_ours, realtime_reference)
    assert usage_counts(realtime_ours[0])["cached_audio_tokens"] == CACHED_AUDIO_TOKENS
    assert usage_counts(realtime_reference[0])["cached_audio_tokens"] == CACHED_AUDIO_TOKENS

    websocket: dict[str, JsonValue] = {
        "type": "response.completed",
        "response": {"usage": usage},
    }
    websocket_ours = WsUsageProcessor.collect_usage_from_responses_ws_results([websocket])
    websocket_reference = (
        REFERENCE_PROCESSORS.ResponsesWebSocketTokenUsageProcessor
        .collect_usage_from_responses_ws_results([websocket])
    )
    compare_usages(websocket_ours, websocket_reference)
    assert usage_counts(websocket_ours[0])["cached_audio_tokens"] == CACHED_AUDIO_TOKENS
    assert usage_counts(websocket_reference[0])["cached_audio_tokens"] == CACHED_AUDIO_TOKENS


def test_processor_empty() -> None:
    ours = BaseTokenUsageProcessor.combine_usage_objects([])
    reference = REFERENCE_PROCESSORS.BaseTokenUsageProcessor.combine_usage_objects([])
    compare_usages([ours], [reference])


def test_processor_missing_usage() -> None:
    events: list[dict[str, JsonValue]] = [
        {"type": "response.done", "response": {}},
        {"type": "response.audio.delta", "delta": "x"},
    ]
    ours = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(events)
    reference = (
        REFERENCE_PROCESSORS.RealtimeAPITokenUsageProcessor
        .collect_usage_from_realtime_stream_results(events)
    )
    compare_usages(ours, reference)
    assert len(ours) == 1
    assert ours[0].total_tokens == ZERO


def test_processor_realtime() -> None:
    ours = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(
        [REALTIME_USAGE]
    )
    compare_usages(ours, REFERENCE_REALTIME_USAGES)
    assert_realtime_counts(ours[0])


def test_processor_tool_use() -> None:
    ours_realtime = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(
        [SERVER_TOOL_REALTIME_EVENT]
    )
    reference_realtime = (
        REFERENCE_PROCESSORS.RealtimeAPITokenUsageProcessor
        .collect_usage_from_realtime_stream_results([SERVER_TOOL_REALTIME_EVENT])
    )
    compare_usages(ours_realtime, reference_realtime)
    realtime_tool_use = ours_realtime[0].server_tool_use
    assert realtime_tool_use is not None
    assert realtime_tool_use.web_search_requests == SERVER_TOOL_REQUESTS

    ours_ws = WsUsageProcessor.collect_usage_from_responses_ws_results([SERVER_TOOL_WS_EVENT])
    reference_ws = (
        REFERENCE_PROCESSORS.ResponsesWebSocketTokenUsageProcessor
        .collect_usage_from_responses_ws_results([SERVER_TOOL_WS_EVENT])
    )
    compare_usages(ours_ws, reference_ws)
    ws_tool_use = ours_ws[0].server_tool_use
    assert ws_tool_use is not None
    assert ws_tool_use.web_search_requests == SERVER_TOOL_REQUESTS


def test_processor_ws_collects() -> None:
    compare_usages(
        WsUsageProcessor.collect_usage_from_responses_ws_results(WS_EVENTS),
        REFERENCE_WS_USAGES,
    )


def test_processor_ws_combines() -> None:
    ours = WsUsageProcessor.collect_and_combine_usage_from_responses_ws_results(WS_EVENTS)
    compare_usages([ours], [REFERENCE_WS_COMBINED])


def test_processor_ws_partitions() -> None:
    ours = WsUsageProcessor.partition_results_by_service_tier(WS_EVENTS)
    assert dict(ours) == {
        tier: tuple(events) for tier, events in REFERENCE_WS_PARTITION.items()
    }


def test_realtime_cost_audio() -> None:
    ours = realtime_stream_cost(
        REALTIME_EVENTS, COMBINED_REALTIME_USAGE, OPENAI, REALTIME_MODEL
    )
    compare_float(ours, REFERENCE_REALTIME_COST)
    assert ours > ZERO


def test_realtime_transcription() -> None:
    ours = realtime_transcription_cost(
        TRANSCRIPTION_EVENTS, OPENAI, TRANSCRIPTION_MODEL
    )
    compare_float(ours, REFERENCE_TRANSCRIPTION_COST)
    assert ours > ZERO


def test_ocr_cost_no_response() -> None:
    with pytest.raises(ValueError, match="response must be of type OCRResponse"):
        ocr_cost(OCR_MODEL, None)


def test_ocr_cost_no_usage_info() -> None:
    response = OCRResponse(pages=PAGES, model=OCR_MODEL, usage_info=None)
    with pytest.raises(ValueError, match="OCR response usage_info is None"):
        ocr_cost(OCR_MODEL, None, response)


def test_processor_logging_object() -> None:
    usage = BaseTokenUsageProcessor.combine_usage_objects(COMBINED_USAGES)
    logging_object = RealtimeAPITokenUsageProcessor.create_logging_realtime_object(
        usage, REALTIME_EVENTS
    )
    compare_usages([logging_object.usage], [REFERENCE_COMBINED_USAGE])
    assert logging_object.results == REALTIME_EVENTS


def test_cost_module_reexports() -> None:
    assert module_ocr_cost is ocr_cost
    assert module_batch_cost_calculator is batch_cost_calculator
    assert module_completion_cost is completion_cost
