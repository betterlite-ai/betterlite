from decimal import Decimal
from typing import TypedDict, cast

import litellm
from litellm import cost_calculator
from litellm.types.utils import CostPerToken, PromptTokensDetailsWrapper, Usage


class RatesInput(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float
    cache_read_input_token_cost: float | None
    cache_creation_cost: float | None


class UsageInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    detail_cache_read: int | None
    detail_cache_creation: int | None
    top_level_cache_read: int | None
    top_level_cache_creation: int | None


class Input(TypedDict):
    model: str | None
    prompt_tokens: int
    completion_tokens: int
    custom_llm_provider: str | None
    custom_cost_per_token: RatesInput | None
    custom_cost_per_second: float | None
    response_time_ms: float | None
    cache_creation_input_tokens: int | None
    cache_read_input_tokens: int | None
    usage: UsageInput | None


class Output(TypedDict):
    prompt_cost: str
    completion_cost: str


def _usage(fields: UsageInput) -> Usage:
    usage = Usage(
        prompt_tokens=fields["prompt_tokens"],
        completion_tokens=fields["completion_tokens"],
    )
    usage.prompt_tokens_details = PromptTokensDetailsWrapper(
        cached_tokens=fields["detail_cache_read"],
        cache_write_tokens=fields["detail_cache_creation"],
    )
    if fields["top_level_cache_read"] is not None:
        usage["cache_read_input_tokens"] = fields["top_level_cache_read"]
    if fields["top_level_cache_creation"] is not None:
        usage["cache_creation_input_tokens"] = fields["top_level_cache_creation"]
    return usage


def _rates(fields: RatesInput) -> CostPerToken:
    rates = CostPerToken(
        input_cost_per_token=fields["input_cost_per_token"],
        output_cost_per_token=fields["output_cost_per_token"],
    )
    if fields["cache_read_input_token_cost"] is not None:
        rates["cache_read_input_token_cost"] = fields["cache_read_input_token_cost"]
    if fields["cache_creation_cost"] is not None:
        rates["cache_creation_input_token_cost"] = fields["cache_creation_cost"]
    return rates


def run(input_value: Input) -> Output:
    _ = litellm.__name__
    usage = None if input_value["usage"] is None else _usage(input_value["usage"])
    prices = input_value["custom_cost_per_token"]
    rates = None if prices is None else _rates(prices)
    prompt_cost, completion_cost = cost_calculator.cost_per_token(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L385
        model=cast("str", input_value["model"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L434
        prompt_tokens=input_value["prompt_tokens"],
        completion_tokens=input_value["completion_tokens"],
        response_time_ms=input_value["response_time_ms"],
        custom_llm_provider=input_value["custom_llm_provider"],
        cache_creation_input_tokens=input_value["cache_creation_input_tokens"],
        cache_read_input_tokens=input_value["cache_read_input_tokens"],
        custom_cost_per_token=rates,
        custom_cost_per_second=input_value["custom_cost_per_second"],
        usage_object=usage,
    )
    return {
        "prompt_cost": format(Decimal(str(prompt_cost)).normalize(), "f"),
        "completion_cost": format(Decimal(str(completion_cost)).normalize(), "f"),
    }
