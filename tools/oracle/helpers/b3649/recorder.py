from datetime import datetime
from decimal import Decimal
from typing import TypedDict

from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import override


class Details(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    litellm_call_id: str
    model_call_details: Details


class LoggedResponse(BaseModel):
    response: str


class Dispatch(BaseModel):
    cache_hit: bool | None = None


class Call(TypedDict):
    cache_hit: bool | None
    response: str


def cost_text(cost: float | None) -> str | None:
    if cost is None:
        return None
    return format(Decimal(repr(cost)).normalize(), "f")


class Recorder(CustomLogger):
    @override
    def __init__(self) -> None:
        self.calls: list[Call] = []

    @override
    async def async_log_success_event(
        self,
        kwargs: dict[str, JsonValue],
        response_obj: dict[str, JsonValue],
        start_time: datetime,
        end_time: datetime,
    ) -> None:
        dispatch = Dispatch.model_validate(kwargs)
        logged = LoggedResponse.model_validate(response_obj)
        self.calls.append({"cache_hit": dispatch.cache_hit, "response": logged.response})

    def only_call(self) -> Call:
        if len(self.calls) != 1:
            raise ValueError(f"the billing sink dispatched {len(self.calls)} times")
        return self.calls[0]


def billing_view(
    logging_obj: Logging,
    response: JsonValue,
    requests: dict[str, JsonValue],
    *,
    cache_hit: bool | None,
) -> dict[str, JsonValue]:
    view = LoggingView.model_validate(logging_obj)
    details = view.model_call_details
    return {
        "model": details.model,
        "custom_llm_provider": details.custom_llm_provider,
        "response_cost": cost_text(details.response_cost),
        "litellm_call_id": view.litellm_call_id,
        "cache_hit": cache_hit,
        "response": response,
        "sse_poller_spawned": tinyfish.sse_poller_spawned(logging_obj),
        "requests": requests,
    }
