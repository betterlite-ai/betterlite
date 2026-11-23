import datetime as dt
import inspect
from inspect import Signature
from typing import Final, Protocol

import pytest
from litellm.constants import DEFAULT_REPLICATE_GPU_PRICE_PER_SECOND as REPLICATE_RATE
from litellm.cost_calculator import completion_cost as reference_completion_cost
from litellm.cost_calculator import cost_per_token as reference_cost_per_token
from litellm.cost_calculator import (
    get_response_cost_from_hidden_params as reference_header_cost,
)
from litellm.cost_calculator import get_usage_object as reference_usage_object
from litellm.cost_calculator import has_hidden_params as reference_has_hidden_params
from litellm.cost_calculator import response_cost_calculator as reference_response_cost
from litellm.exceptions import ModelNotMappedError as ReferenceNotMapped
from litellm.litellm_core_utils.litellm_logging import Logging as ReferenceLogging
from litellm.types.utils import EmbeddingResponse as ReferenceEmbedding
from litellm.types.utils import ModelResponse as ReferenceChat
from litellm.types.utils import Usage as ReferenceUsage
from pydantic import BaseModel, JsonValue
from typing_extensions import override

from betterlite import (
    completion_cost,
    cost_per_token,
    get_replicate_completion_pricing,
    get_response_cost_from_hidden_params,
    get_usage_object,
    has_hidden_params,
    response_cost_calculator,
)
from betterlite._cost.cost_logging import BilledTokenRates, CostLogging
from betterlite.exceptions import BadRequestError, ModelNotMappedError
from betterlite.types.utils import EmbeddingResponse, ModelResponse, Usage

CHAT_MODEL: Final = "gpt-4o"
CACHED_MODEL: Final = "claude-haiku-4-5"
TIERED_MODEL: Final = "dashscope/qwen-flash"
FLEX_MODEL: Final = "gpt-5"
REGION_MODEL: Final = "anthropic.claude-3-5-sonnet-20241022-v2:0"
SECONDS_MODEL: Final = "*/1-month-commitment/cohere.command-light-text-v14"
EMBEDDING_MODEL: Final = "text-embedding-3-small"
UNKNOWN_MODEL: Final = "unknown-model-xyz"
OPENAI: Final = "openai"
ANTHROPIC: Final = "anthropic"
BEDROCK: Final = "bedrock"
REGION: Final = "us-west-2"
COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
RELATIVE: Final = 1e-12
ZERO_COST: Final = 0.0
HEADER_COST: Final = 0.25
SECONDS: Final = 2.0
REPLICATE_SECONDS: Final = 2500.0
STAMP_START: Final = 1.0
STAMP_END: Final = 2.0
UNMAPPED_MATCH: Final = "This model isn't mapped yet"
PROVIDER_MATCH: Final = "LLM Provider NOT provided"
REFERENCE_ERROR: Final[tuple[str, str]] = ("litellm.exceptions", "BadRequestError")
MODEL_MISSING_MATCH: Final = "Model is None"
HIDDEN: Final[dict[str, JsonValue]] = {"additional_headers": {COST_HEADER: HEADER_COST}}
STRING_HEADER: Final[dict[str, JsonValue]] = {"additional_headers": {COST_HEADER: "0.00042"}}
EMPTY_PARAMS: Final[dict[str, JsonValue]] = {}
EMPTY_HEADERS: Final[dict[str, JsonValue]] = {"additional_headers": {}}
EMPTY_MESSAGES: Final[list[JsonValue]] = []
BREAKDOWN_KWARGS: Final[tuple[str, ...]] = (
    "input_cost",
    "output_cost",
    "total_cost",
    "cost_for_built_in_tools_cost_usd_dollar",
    "additional_costs",
    "original_cost",
    "discount_percent",
    "discount_amount",
    "margin_percent",
    "margin_fixed_amount",
    "margin_total_amount",
    "cache_read_cost",
    "cache_creation_cost",
    "reasoning_cost",
    "service_tier",
    "data_residency",
    "vertex_location",
    "billed_token_rates",
)


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


class Recorder(CostLogging):
    model: str | None
    values: dict[str, JsonValue | BilledTokenRates]

    def __init__(self, model: str | None) -> None:
        self.model = model
        self.values = {}

    @override
    def set_cost_breakdown(self, **values: JsonValue | BilledTokenRates) -> None:
        self.values = values


class BreakdownValues(BaseModel):
    input_cost: float
    output_cost: float
    total_cost: float
    cost_for_built_in_tools_cost_usd_dollar: float  # ast-grep-ignore: no-long-names
    additional_costs: dict[str, JsonValue] | None
    original_cost: float
    discount_percent: float
    discount_amount: float
    margin_percent: float
    margin_fixed_amount: float
    margin_total_amount: float
    cache_read_cost: float
    cache_creation_cost: float
    reasoning_cost: float
    service_tier: str | None
    data_residency: str | None
    vertex_location: str | None
    billed_token_rates: BilledTokenRates


def signature_rows(signature: Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        view: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(view.default)))
    return rows


def assert_pair_matches(ours: tuple[float, float], reference: tuple[float, float]) -> None:
    assert ours[0] == pytest.approx(reference[0], rel=RELATIVE)
    assert ours[1] == pytest.approx(reference[1], rel=RELATIVE)


def our_chat(model: str, prompt: int, completion: int) -> ModelResponse:
    usage = Usage(
        prompt_tokens=prompt, completion_tokens=completion, total_tokens=prompt + completion,
    )
    return ModelResponse(model=model, usage=usage)


def reference_chat(model: str, prompt: int, completion: int) -> ReferenceChat:
    usage = ReferenceUsage(
        prompt_tokens=prompt, completion_tokens=completion, total_tokens=prompt + completion,
    )
    return ReferenceChat(model=model, usage=usage)


def our_cached_chat(model: str) -> ModelResponse:
    usage = Usage(
        prompt_tokens=100, completion_tokens=10, total_tokens=110,
        cache_read_input_tokens=50, cache_creation_input_tokens=20,
    )
    return ModelResponse(model=model, usage=usage)


def reference_cached_chat(model: str) -> ReferenceChat:
    usage = ReferenceUsage(
        prompt_tokens=100, completion_tokens=10, total_tokens=110,
        cache_read_input_tokens=50, cache_creation_input_tokens=20,
    )
    return ReferenceChat(model=model, usage=usage)


# The tables below are LiteLLM's rows at the pin; mypy rejects naming its Any callables.
PARAM_KIND: Final = "POSITIONAL_OR_KEYWORD"
REQUIRED: Final = "<class 'inspect._empty'>"
COST_PER_TOKEN_SIGNATURE: Final[tuple[tuple[str, str, str], ...]] = (
    ("model", PARAM_KIND, "''"), ("prompt_tokens", PARAM_KIND, "0"),
    ("completion_tokens", PARAM_KIND, "0"), ("response_time_ms", PARAM_KIND, "0.0"),
    ("custom_llm_provider", PARAM_KIND, "None"), ("region_name", PARAM_KIND, "None"),
    ("prompt_characters", PARAM_KIND, "None"),
    ("completion_characters", PARAM_KIND, "None"),
    ("cache_creation_input_tokens", PARAM_KIND, "0"),
    ("cache_read_input_tokens", PARAM_KIND, "0"),
    ("custom_cost_per_token", PARAM_KIND, "None"),
    ("custom_cost_per_second", PARAM_KIND, "None"),
    ("number_of_queries", PARAM_KIND, "None"), ("usage_object", PARAM_KIND, "None"),
    ("rerank_billed_units", PARAM_KIND, "None"), ("call_type", PARAM_KIND, "'completion'"),
    ("audio_transcription_file_duration", PARAM_KIND, "0.0"),
    ("service_tier", PARAM_KIND, "None"), ("data_residency", PARAM_KIND, "None"),
    ("vertex_location", PARAM_KIND, "None"), ("response", PARAM_KIND, "None"),
    ("request_model", PARAM_KIND, "None"), ("custom_model_info", PARAM_KIND, "None"),
)
COMPLETION_COST_SIGNATURE: Final[tuple[tuple[str, str, str], ...]] = (
    ("completion_response", PARAM_KIND, "None"), ("model", PARAM_KIND, "None"),
    ("prompt", PARAM_KIND, "''"), ("messages", PARAM_KIND, "[]"),
    ("completion", PARAM_KIND, "''"), ("total_time", PARAM_KIND, "0.0"),
    ("call_type", PARAM_KIND, "None"), ("custom_llm_provider", PARAM_KIND, "None"),
    ("region_name", PARAM_KIND, "None"), ("size", PARAM_KIND, "None"),
    ("quality", PARAM_KIND, "None"), ("n", PARAM_KIND, "None"),
    ("custom_cost_per_token", PARAM_KIND, "None"),
    ("custom_cost_per_second", PARAM_KIND, "None"),
    ("optional_params", PARAM_KIND, "None"), ("custom_pricing", PARAM_KIND, "None"),
    ("base_model", PARAM_KIND, "None"),
    ("standard_built_in_tools_params", PARAM_KIND, "None"),
    ("litellm_model_name", PARAM_KIND, "None"), ("router_model_id", PARAM_KIND, "None"),
    ("litellm_logging_obj", PARAM_KIND, "None"), ("service_tier", PARAM_KIND, "None"),
    ("data_residency", PARAM_KIND, "None"), ("vertex_location", PARAM_KIND, "None"),
)
RESPONSE_COST_SIGNATURE: Final[tuple[tuple[str, str, str], ...]] = (
    ("response_object", PARAM_KIND, REQUIRED), ("model", PARAM_KIND, REQUIRED),
    ("custom_llm_provider", PARAM_KIND, REQUIRED), ("call_type", PARAM_KIND, REQUIRED),
    ("optional_params", PARAM_KIND, REQUIRED), ("cache_hit", PARAM_KIND, "None"),
    ("base_model", PARAM_KIND, "None"), ("custom_pricing", PARAM_KIND, "None"),
    ("prompt", PARAM_KIND, "''"), ("standard_built_in_tools_params", PARAM_KIND, "None"),
    ("litellm_model_name", PARAM_KIND, "None"), ("router_model_id", PARAM_KIND, "None"),
    ("litellm_logging_obj", PARAM_KIND, "None"), ("service_tier", PARAM_KIND, "None"),
    ("data_residency", PARAM_KIND, "None"), ("vertex_location", PARAM_KIND, "None"),
    ("region_name", PARAM_KIND, "None"),
)


def test_cost_per_token_signature() -> None:
    assert tuple(signature_rows(inspect.signature(cost_per_token))) == COST_PER_TOKEN_SIGNATURE


def test_completion_cost_signature() -> None:
    assert tuple(signature_rows(inspect.signature(completion_cost))) == COMPLETION_COST_SIGNATURE


def test_response_cost_signature() -> None:
    signature = inspect.signature(response_cost_calculator)
    assert tuple(signature_rows(signature)) == RESPONSE_COST_SIGNATURE


def test_cost_per_token_plain() -> None:
    ours = cost_per_token(model=CHAT_MODEL, prompt_tokens=10, completion_tokens=5)
    reference = reference_cost_per_token(model=CHAT_MODEL, prompt_tokens=10, completion_tokens=5)
    assert_pair_matches(ours, reference)


def test_cost_per_token_cached() -> None:
    ours = cost_per_token(CACHED_MODEL, 100, 10, custom_llm_provider=ANTHROPIC,
                          cache_read_input_tokens=50, cache_creation_input_tokens=20)
    assert_pair_matches(ours, reference_cost_per_token(
        CACHED_MODEL, 100, 10, custom_llm_provider=ANTHROPIC,
        cache_read_input_tokens=50, cache_creation_input_tokens=20,
    ))


def test_cost_per_token_custom() -> None:
    custom = {"input_cost_per_token": 0.5, "output_cost_per_token": 1.5}
    ours = cost_per_token(CHAT_MODEL, 10, 5, custom_cost_per_token=custom)
    assert_pair_matches(ours, reference_cost_per_token(
        CHAT_MODEL, 10, 5,
        custom_cost_per_token={"input_cost_per_token": 0.5, "output_cost_per_token": 1.5},
    ))


def test_cost_per_token_tiered() -> None:
    ours = cost_per_token(model=TIERED_MODEL, prompt_tokens=300000, completion_tokens=10)
    reference = reference_cost_per_token(
        model=TIERED_MODEL, prompt_tokens=300000, completion_tokens=10
    )
    assert_pair_matches(ours, reference)


def test_cost_per_token_flex_tier() -> None:
    ours = cost_per_token(model=FLEX_MODEL, prompt_tokens=1000, completion_tokens=100,
                          service_tier="flex")
    reference = reference_cost_per_token(
        model=FLEX_MODEL, prompt_tokens=1000, completion_tokens=100, service_tier="flex",
    )
    assert_pair_matches(ours, reference)


def test_cost_per_token_seconds() -> None:
    ours = cost_per_token(model=SECONDS_MODEL, custom_llm_provider=BEDROCK,
                          response_time_ms=2000.0)
    reference = reference_cost_per_token(
        model=SECONDS_MODEL, custom_llm_provider=BEDROCK, response_time_ms=2000.0,
    )
    assert_pair_matches(ours, reference)


def test_cost_per_token_region() -> None:
    ours = cost_per_token(
        model=REGION_MODEL, prompt_tokens=100, completion_tokens=10,
        custom_llm_provider=BEDROCK, region_name=REGION,
    )
    assert_pair_matches(ours, reference_cost_per_token(
        model=REGION_MODEL, prompt_tokens=100, completion_tokens=10,
        custom_llm_provider=BEDROCK, region_name=REGION,
    ))


def test_cost_per_token_unmapped() -> None:
    with pytest.raises(ModelNotMappedError, match=UNMAPPED_MATCH) as ours_error:
        cost_per_token(model=UNKNOWN_MODEL, custom_llm_provider=OPENAI)
    with pytest.raises(ReferenceNotMapped, match=UNMAPPED_MATCH) as reference_error:
        reference_cost_per_token(model=UNKNOWN_MODEL, custom_llm_provider=OPENAI)
    assert str(ours_error.value) == str(reference_error.value)


def test_cost_per_token_provider() -> None:
    with pytest.raises(BadRequestError, match=PROVIDER_MATCH) as ours_error:
        cost_per_token(model=UNKNOWN_MODEL)
    with pytest.raises(Exception, match=PROVIDER_MATCH) as reference_error:
        reference_cost_per_token(model=UNKNOWN_MODEL)
    assert (reference_error.type.__module__, reference_error.type.__name__) == REFERENCE_ERROR
    assert str(ours_error.value) == str(reference_error.value)


def test_completion_cost_chat() -> None:
    ours = completion_cost(our_chat(CHAT_MODEL, 10, 5))
    reference = reference_completion_cost(reference_chat(CHAT_MODEL, 10, 5))
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_completion_cost_cached() -> None:
    ours = completion_cost(our_cached_chat(CACHED_MODEL))
    assert ours == pytest.approx(reference_completion_cost(reference_cached_chat(CACHED_MODEL)),
                                 rel=RELATIVE)


def test_completion_cost_embedding() -> None:
    usage = Usage(prompt_tokens=1000, completion_tokens=0, total_tokens=1000)
    reference_usage = ReferenceUsage(prompt_tokens=1000, completion_tokens=0, total_tokens=1000)
    ours = completion_cost(
        EmbeddingResponse(model=EMBEDDING_MODEL, usage=usage), model=EMBEDDING_MODEL,
        call_type="embedding",
    )
    reference = reference_completion_cost(
        ReferenceEmbedding(model=EMBEDDING_MODEL, usage=reference_usage),
        model=EMBEDDING_MODEL, call_type="embedding",
    )
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_completion_cost_custom() -> None:
    custom = {"input_cost_per_token": 0.5, "output_cost_per_token": 1.5}
    ours = completion_cost(our_chat(CHAT_MODEL, 10, 5), custom_cost_per_token=custom)
    assert ours == pytest.approx(reference_completion_cost(
        reference_chat(CHAT_MODEL, 10, 5),
        custom_cost_per_token={"input_cost_per_token": 0.5, "output_cost_per_token": 1.5},
    ), rel=RELATIVE)


def test_completion_cost_tiered() -> None:
    ours = completion_cost(our_chat(TIERED_MODEL, 300000, 10))
    reference = reference_completion_cost(reference_chat(TIERED_MODEL, 300000, 10))
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_completion_cost_flex_tier() -> None:
    ours = completion_cost(our_chat(FLEX_MODEL, 1000, 100), service_tier="flex")
    assert ours == pytest.approx(
        reference_completion_cost(reference_chat(FLEX_MODEL, 1000, 100), service_tier="flex"),
        rel=RELATIVE,
    )


def test_completion_cost_seconds() -> None:
    ours = completion_cost(model=SECONDS_MODEL, custom_llm_provider=BEDROCK, total_time=SECONDS)
    reference = reference_completion_cost(
        model=SECONDS_MODEL, custom_llm_provider=BEDROCK, total_time=SECONDS,
    )
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_completion_cost_region() -> None:
    ours = completion_cost(
        our_chat(REGION_MODEL, 100, 10), model=REGION_MODEL,
        custom_llm_provider=BEDROCK, region_name=REGION,
    )
    reference = reference_completion_cost(
        reference_chat(REGION_MODEL, 100, 10), model=REGION_MODEL,
        custom_llm_provider=BEDROCK, region_name=REGION,
    )
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_completion_cost_unmapped() -> None:
    with pytest.raises(ModelNotMappedError, match=UNMAPPED_MATCH) as ours_error:
        completion_cost(our_chat(UNKNOWN_MODEL, 1, 1), custom_llm_provider=OPENAI)
    with pytest.raises(ReferenceNotMapped, match=UNMAPPED_MATCH) as reference_error:
        reference_completion_cost(
            reference_chat(UNKNOWN_MODEL, 1, 1), custom_llm_provider=OPENAI,
        )
    assert str(ours_error.value) == str(reference_error.value)


def test_completion_cost_no_model() -> None:
    with pytest.raises(ValueError, match=MODEL_MISSING_MATCH) as ours_error:
        completion_cost()
    with pytest.raises(ValueError, match=MODEL_MISSING_MATCH) as reference_error:
        reference_completion_cost()
    assert str(ours_error.value) == str(reference_error.value)


def test_response_cost_cache_hit() -> None:
    ours = response_cost_calculator(
        our_chat(CHAT_MODEL, 10, 5), CHAT_MODEL, OPENAI, "completion", EMPTY_PARAMS,
        cache_hit=True,
    )
    reference = reference_response_cost(
        reference_chat(CHAT_MODEL, 10, 5), CHAT_MODEL, OPENAI, "completion", EMPTY_PARAMS,
        cache_hit=True,
    )
    assert ours == ZERO_COST
    assert ours == reference


def test_response_cost_header() -> None:
    usage = Usage(prompt_tokens=10, completion_tokens=5, total_tokens=15)
    response = ModelResponse(model=CHAT_MODEL, usage=usage, hidden_params=HIDDEN)
    ours = response_cost_calculator(response, CHAT_MODEL, OPENAI, "completion", EMPTY_PARAMS)
    assert ours == pytest.approx(HEADER_COST, rel=RELATIVE)
    assert ours == reference_header_cost(HIDDEN)


def test_response_cost_computed() -> None:
    ours = response_cost_calculator(our_chat(CHAT_MODEL, 10, 5), CHAT_MODEL, OPENAI,
                                    "completion", EMPTY_PARAMS)
    reference = reference_response_cost(
        reference_chat(CHAT_MODEL, 10, 5), CHAT_MODEL, OPENAI, "completion", EMPTY_PARAMS
    )
    assert ours == pytest.approx(reference, rel=RELATIVE)


def test_response_cost_breakdown() -> None:
    recorder = Recorder(model=CACHED_MODEL)
    ours = completion_cost(our_cached_chat(CACHED_MODEL), litellm_logging_obj=recorder)
    logger = ReferenceLogging(
        model=CACHED_MODEL, messages=EMPTY_MESSAGES, stream=False, call_type="completion",
        start_time=dt.datetime.now(tz=dt.UTC), litellm_call_id="b8063-cost",
        function_id="b8063-cost",
    )
    reference = reference_completion_cost(
        reference_cached_chat(CACHED_MODEL), litellm_logging_obj=logger
    )
    assert ours == pytest.approx(reference, rel=RELATIVE)
    assert sorted(recorder.values) == sorted(BREAKDOWN_KWARGS)
    values = BreakdownValues.model_validate(recorder.values)
    stored = logger.cost_breakdown
    assert stored is not None
    assert values.input_cost == pytest.approx(stored["input_cost"], rel=RELATIVE)
    assert values.output_cost == pytest.approx(stored["output_cost"], rel=RELATIVE)
    assert values.total_cost == pytest.approx(stored["total_cost"], rel=RELATIVE)
    assert values.cost_for_built_in_tools_cost_usd_dollar == pytest.approx(
        stored["tool_usage_cost"], rel=RELATIVE
    )
    assert values.cache_read_cost == pytest.approx(stored["cache_read_cost"], rel=RELATIVE)
    assert values.cache_creation_cost == pytest.approx(
        stored["cache_creation_cost"], rel=RELATIVE
    )
    assert values.original_cost == pytest.approx(stored["original_cost"], rel=RELATIVE)
    assert values.discount_percent == pytest.approx(stored["discount_percent"], rel=RELATIVE)
    assert values.discount_amount == pytest.approx(stored["discount_amount"], rel=RELATIVE)
    assert values.margin_percent == pytest.approx(stored["margin_percent"], rel=RELATIVE)
    assert values.margin_fixed_amount == pytest.approx(
        stored["margin_fixed_amount"], rel=RELATIVE
    )
    assert values.margin_total_amount == pytest.approx(
        stored["margin_total_amount"], rel=RELATIVE
    )
    assert values.service_tier == stored["service_tier"]
    assert values.data_residency == stored["data_residency"]
    assert values.vertex_location == stored["vertex_location"]
    rates = values.billed_token_rates
    reference_rates = logger.billed_token_rates
    assert reference_rates is not None
    assert (
        rates.input_cost_per_token,
        rates.output_cost_per_token,
        rates.cache_read_input_token_cost,
        rates.cache_read_input_audio_token_cost,
        rates.cache_creation_input_token_cost,
        rates.cache_creation_input_token_cost_above_1hr,
        rates.output_cost_per_reasoning_token,
    ) == pytest.approx(
        (
            reference_rates.input_cost_per_token,
            reference_rates.output_cost_per_token,
            reference_rates.cache_read_input_token_cost,
            reference_rates.cache_read_input_audio_token_cost,
            reference_rates.cache_creation_input_token_cost,
            reference_rates.cache_creation_input_token_cost_above_1hr,
            reference_rates.output_cost_per_reasoning_token,
        ),
        rel=RELATIVE,
    )


def assert_usage_matches(ours: Usage | None, reference: ReferenceUsage | None) -> None:
    assert ours is not None
    assert reference is not None
    assert (ours.prompt_tokens, ours.completion_tokens, ours.total_tokens) == (
        reference.prompt_tokens, reference.completion_tokens, reference.total_tokens,
    )


def test_usage_object_present() -> None:
    ours = get_usage_object(our_chat(CHAT_MODEL, 10, 5))
    reference = reference_usage_object(reference_chat(CHAT_MODEL, 10, 5))
    assert_usage_matches(ours, reference)


def test_usage_object_absent() -> None:
    assert get_usage_object({"model": CHAT_MODEL}) is None
    assert reference_usage_object({"model": CHAT_MODEL}) is None


def test_usage_object_mapping() -> None:
    usage: Final[dict[str, JsonValue]] = {
        "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6},
    }
    assert_usage_matches(get_usage_object(usage), reference_usage_object(usage))


def test_usage_object_response_api() -> None:
    response: Final[dict[str, JsonValue]] = {
        "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
    }
    assert_usage_matches(get_usage_object(response), reference_usage_object(response))


def test_hidden_params_response() -> None:
    assert has_hidden_params(our_chat(CHAT_MODEL, 10, 5))
    assert has_hidden_params(EMPTY_PARAMS) is False
    assert reference_has_hidden_params(reference_chat(CHAT_MODEL, 10, 5))
    assert reference_has_hidden_params(EMPTY_PARAMS) is False


def test_hidden_params_cost() -> None:
    assert get_response_cost_from_hidden_params(HIDDEN) == HEADER_COST
    assert get_response_cost_from_hidden_params(STRING_HEADER) == float("0.00042")
    assert get_response_cost_from_hidden_params(EMPTY_HEADERS) is None
    assert get_response_cost_from_hidden_params(EMPTY_PARAMS) is None
    assert reference_header_cost(HIDDEN) == HEADER_COST
    assert reference_header_cost(STRING_HEADER) == float("0.00042")
    assert reference_header_cost(EMPTY_HEADERS) is None
    assert reference_header_cost(EMPTY_PARAMS) is None


def test_replicate_pricing_time() -> None:
    ours = get_replicate_completion_pricing(EMPTY_PARAMS, REPLICATE_SECONDS)
    assert ours == pytest.approx(REPLICATE_RATE * REPLICATE_SECONDS / 1000, rel=RELATIVE)


def test_replicate_pricing_stamps() -> None:
    stamps: Final[dict[str, JsonValue]] = {"created": STAMP_START, "ended": STAMP_END}
    ours = get_replicate_completion_pricing(stamps)
    assert ours == pytest.approx(
        REPLICATE_RATE * (STAMP_END - STAMP_START) / 1000, rel=RELATIVE
    )
