from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from litellm.types.passthrough_endpoints.tinyfish import is_allowed_tinyfish_endpoint
from pydantic import BaseModel, ConfigDict, JsonValue

START = datetime(2024, 1, 1, tzinfo=UTC)
END = datetime(2024, 1, 1, 0, 0, 5, tzinfo=UTC)


class Run(TypedDict, total=False):
    run_id: str | None
    status: str | None
    num_of_steps: int | None
    result: JsonValue
    error: JsonValue
    type: str | None


class BillingInput(TypedDict):
    function: Literal["run_billing"]
    run: Run | None


class EndpointInput(TypedDict):
    function: Literal["endpoint"]
    method: str
    path: str


class ApiBaseInput(TypedDict):
    function: Literal["api_base"]


class CostPerStepInput(TypedDict):
    function: Literal["cost_per_step"]


class SseInput(TypedDict):
    function: Literal["sse_run_id"]
    frames: list[int]


Input: TypeAlias = BillingInput | EndpointInput | ApiBaseInput | CostPerStepInput | SseInput


class Billing(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class LoggedRun(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    model_call_details: Billing


def cost_text(cost: float | None) -> str | None:
    if cost is None:
        return None
    return format(Decimal(repr(cost)).normalize(), "f")


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=START,
        litellm_call_id="call-1",
        function_id="function-1",
    )


def billing_value(call: BillingInput) -> dict[str, JsonValue]:
    logging_obj = make_logging()
    tinyfish.TinyFishPassthroughLoggingHandler.tinyfish_passthrough_handler(
        httpx_response=httpx.Response(200),
        response_body=call["run"],
        logging_obj=logging_obj,
        url_route="https://agent.tinyfish.ai/v1/automation/run",
        result="",
        start_time=START,
        end_time=END,
        cache_hit=False,
        request_body={},
    )
    details = LoggedRun.model_validate(logging_obj).model_call_details
    return {
        "model": details.model,
        "custom_llm_provider": details.custom_llm_provider,
        "response_cost": cost_text(details.response_cost),
    }


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "run_billing":
            return billing_value(input_value)
        case "endpoint":
            return is_allowed_tinyfish_endpoint(input_value["method"], input_value["path"])
        case "api_base":
            return tinyfish.resolve_tinyfish_agent_api_base()
        case "cost_per_step":
            rate = tinyfish.resolve_tinyfish_cost_per_step()
            return format(Decimal(repr(rate)).normalize(), "f")
        case "sse_run_id":
            frames = bytes(input_value["frames"])
            return tinyfish.run_id_from_sse_frames(frames)
