import logging
from dataclasses import dataclass
from typing import Final, Protocol

from pydantic import JsonValue

from betterlite._cost.wire import CostAnswerWire, RatesWire

LOGGER: Final = logging.getLogger("LiteLLM")


# LiteLLM's BilledTokenRates: the per-token rates the charge used, in the order it declares them.
@dataclass(frozen=True)
class BilledTokenRates:
    input_cost_per_token: float
    output_cost_per_token: float
    cache_read_input_token_cost: float
    cache_read_input_audio_token_cost: float  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost: float  # ast-grep-ignore: no-long-names
    cache_creation_input_token_cost_above_1hr: float  # ast-grep-ignore: no-long-names
    output_cost_per_reasoning_token: float  # ast-grep-ignore: no-long-names


# What completion_cost reads off litellm_logging_obj; the object itself stays in Python.
class CostLogging(Protocol):
    model: str | None

    def set_cost_breakdown(self, **values: JsonValue | BilledTokenRates) -> None: ...


# LiteLLM's request_model_for_cost: the deployment's model, which the router detection prices on.
def logging_model(logging_obj: CostLogging | None) -> str | None:
    if logging_obj is None:
        return None
    return logging_obj.model


def billed_rates(rates: RatesWire) -> BilledTokenRates:
    return BilledTokenRates(
        input_cost_per_token=rates["input_cost_per_token"],
        output_cost_per_token=rates["output_cost_per_token"],
        cache_read_input_token_cost=rates["cache_read_input_token_cost"],
        cache_read_input_audio_token_cost=rates["cache_read_input_audio_token_cost"],
        cache_creation_input_token_cost=rates["cache_creation_input_token_cost"],
        cache_creation_input_token_cost_above_1hr=rates[
            "cache_creation_input_token_cost_above_1hr"
        ],
        output_cost_per_reasoning_token=rates["output_cost_per_reasoning_token"],
    )


# _store_cost_breakdown_in_logging_obj: the fields LiteLLM passes, a broken logger never failing the charge.
def store_breakdown(logging_obj: CostLogging | None, answer: CostAnswerWire) -> None:
    if logging_obj is None:
        return
    breakdown = answer["breakdown"]
    if breakdown is None:
        return
    rates = answer["billed_token_rates"]
    try:
        logging_obj.set_cost_breakdown(
            input_cost=breakdown["input_cost"],
            output_cost=breakdown["output_cost"],
            total_cost=breakdown["total_cost"],
            cost_for_built_in_tools_cost_usd_dollar=breakdown["tool_usage_cost"],
            additional_costs=breakdown.get("additional_costs"),
            original_cost=breakdown.get("original_cost"),
            discount_percent=breakdown.get("discount_percent"),
            discount_amount=breakdown.get("discount_amount"),
            margin_percent=breakdown.get("margin_percent"),
            margin_fixed_amount=breakdown.get("margin_fixed_amount"),
            margin_total_amount=breakdown.get("margin_total_amount"),
            cache_read_cost=breakdown.get("cache_read_cost"),
            cache_creation_cost=breakdown.get("cache_creation_cost"),
            reasoning_cost=breakdown.get("reasoning_cost"),
            service_tier=breakdown.get("service_tier"),
            data_residency=breakdown.get("data_residency"),
            vertex_location=breakdown.get("vertex_location"),
            billed_token_rates=None if rates is None else billed_rates(rates),
        )
    except Exception as breakdown_error:
        LOGGER.debug("Error storing cost breakdown: %s", breakdown_error)
