from typing import Literal, NotRequired, TypedDict

import litellm
from litellm import cost_calculator
from litellm.types.utils import (
    CostPerToken,
    ModelResponse,
    PromptTokensDetailsWrapper,
    Usage,
)


class Entry(TypedDict):
    litellm_provider: str
    mode: str
    input_cost_per_token: float
    output_cost_per_token: float
    cache_read_input_token_cost: NotRequired[float | None]


class UsageInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    cached_tokens: int | None


class Rates(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float


class Split(TypedDict):
    percentage: float | None
    fixed_amount: float | None


class Input(TypedDict):
    function: Literal["completion_cost", "response_cost_calculator"]
    model: str
    custom_llm_provider: str | None
    region_name: str | None
    catalog: dict[str, Entry]
    usage: UsageInput
    custom_price: Rates | None
    cache_hit: bool | None
    header_cost: float | None
    discount: dict[str, float]
    margin: dict[str, float | Split]


class Output(TypedDict):
    cost: float


def margin_config(margin: dict[str, float | Split]) -> dict[str, float | dict[str, float]]:
    config: dict[str, float | dict[str, float]] = {}
    for provider, entry in margin.items():
        if isinstance(entry, float | int):
            config[provider] = entry
            continue
        fields: dict[str, float] = {}
        if entry["percentage"] is not None:
            fields["percentage"] = entry["percentage"]
        if entry["fixed_amount"] is not None:
            fields["fixed_amount"] = entry["fixed_amount"]
        config[provider] = fields
    return config


def priced_response(input_value: Input) -> ModelResponse:
    fields = input_value["usage"]
    usage = Usage(
        prompt_tokens=fields["prompt_tokens"],
        completion_tokens=fields["completion_tokens"],
    )
    if fields["cached_tokens"] is not None:
        usage.prompt_tokens_details = PromptTokensDetailsWrapper(
            cached_tokens=fields["cached_tokens"]
        )
    header = input_value["header_cost"]
    hidden_params = (
        None
        if header is None
        else {"additional_headers": {"llm_provider-x-litellm-response-cost": header}}
    )
    return ModelResponse(model=input_value["model"], usage=usage, hidden_params=hidden_params)


def run(input_value: Input) -> Output:
    for model, entry in input_value["catalog"].items():
        record = {key: value for key, value in entry.items() if value is not None}
        litellm.register_model({model: record})
    litellm.cost_discount_config = input_value["discount"]
    litellm.cost_margin_config = margin_config(input_value["margin"])
    response = priced_response(input_value)
    prices = input_value["custom_price"]
    rates = (
        None
        if prices is None
        else CostPerToken(
            input_cost_per_token=prices["input_cost_per_token"],
            output_cost_per_token=prices["output_cost_per_token"],
        )
    )
    if input_value["function"] == "completion_cost":
        cost = cost_calculator.completion_cost(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1332
            completion_response=response,
            model=input_value["model"],
            custom_llm_provider=input_value["custom_llm_provider"],
            region_name=input_value["region_name"],
            custom_cost_per_token=rates,
        )
    else:
        cost = cost_calculator.response_cost_calculator(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1979
            response_object=response,
            model=input_value["model"],
            custom_llm_provider=input_value["custom_llm_provider"],
            call_type="completion",
            optional_params={},
            cache_hit=input_value["cache_hit"],
            region_name=input_value["region_name"],
        )
    return {"cost": cost}
