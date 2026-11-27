import importlib
import sys
from collections.abc import Awaitable, Callable
from datetime import timezone
from types import ModuleType
from typing import Literal

from fastapi import Request
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.lens import inference
from litellm.types.utils import CostPerToken, ModelResponse
from pydantic import JsonValue

from .fixtures import Clock, DeploymentCall, StopEarlyError, UsageCall

BILLING_MODULE = "litellm.proxy.lens.billing"
DATETIME_MODULE = "datetime"
LITELLM_MODULE = "litellm"
PROXY_MODULE = "litellm.proxy.proxy_server"


def response() -> ModelResponse:
    choices: list[JsonValue] = [{"index": 0, "message": {"role": "assistant", "content": "{}"}}]
    usage: dict[str, JsonValue] = {"prompt_tokens": 2, "completion_tokens": 4, "total_tokens": 6}
    return ModelResponse(id="x", created=1, model="lens-model", choices=choices, usage=usage)


class Recorder:
    def __init__(self, mode: Literal["reserve", "settle", "body"], cost: float | None) -> None:
        self.mode = mode
        self.cost = cost
        self.data: dict[str, JsonValue] | None = None

    async def complete(
        self,
        key_id: str,
        data: dict[str, JsonValue],
        reserve: Callable[[], Awaitable[None]],
        incoming: Request,
    ) -> tuple[ModelResponse, float | None]:
        self.key_id = key_id
        self.incoming = incoming
        self.data = data
        if self.mode == "reserve":
            await reserve()
            raise StopEarlyError
        if self.mode == "body":
            raise StopEarlyError
        return (response(), self.cost)


class Key:
    def __init__(self, team_id: str | None) -> None:
        self.team_id = team_id

    async def validate(self, key_id: str | None) -> UserAPIKeyAuth | None:
        self.key_id = key_id
        return UserAPIKeyAuth(team_id=self.team_id)


class Router:
    def __init__(self, deployments: list[DeploymentCall]) -> None:
        self.deployments = deployments

    def get_model_list(self, model_name: str, team_id: str | None) -> list[DeploymentCall]:
        self.query = (model_name, team_id)
        return self.deployments


class ProxyServer(ModuleType):
    def __init__(self, router: Router) -> None:
        super().__init__(PROXY_MODULE)
        self.llm_router = router


class BillingModule(ModuleType):
    def __init__(self, recorder: Recorder, key: Key) -> None:
        super().__init__(BILLING_MODULE)
        self.complete = recorder.complete
        self.validate_key = key.validate


class DatetimeModule(ModuleType):
    def __init__(self) -> None:
        super().__init__(DATETIME_MODULE)
        self.datetime = Clock
        self.timezone = timezone


class LiteLLMModule(ModuleType):
    def __init__(self, entries: dict[str, dict[str, JsonValue]], usage: UsageCall) -> None:
        super().__init__(LITELLM_MODULE)
        self.entries = entries
        self.usage = usage

    def get_model_info(
        self,
        model: str,
        _custom_llm_provider: str | None = None,
        _api_base: str | None = None,
        _api_key: str | None = None,
    ) -> dict[str, JsonValue]:
        return self.entries[model]

    def completion_cost(
        self,
        completion_response: ModelResponse,
        model: str | None = None,
        custom_cost_per_token: CostPerToken | None = None,
    ) -> float:
        if custom_cost_per_token is None:
            message = f"the oracle bills only custom rates, not {model} on {completion_response.id}"
            raise ValueError(message)
        input_rate = custom_cost_per_token["input_cost_per_token"]
        output_rate = custom_cost_per_token["output_cost_per_token"]
        return self.usage["prompt_tokens"] * input_rate + self.usage["completion_tokens"] * output_rate


def reload_inference(litellm_module: LiteLLMModule, billing: BillingModule, clock: DatetimeModule) -> None:
    saved: dict[str, ModuleType | None] = {
        LITELLM_MODULE: sys.modules.get(LITELLM_MODULE),
        BILLING_MODULE: sys.modules.get(BILLING_MODULE),
        DATETIME_MODULE: sys.modules.get(DATETIME_MODULE),
    }
    sys.modules[LITELLM_MODULE] = litellm_module
    sys.modules[BILLING_MODULE] = billing
    sys.modules[DATETIME_MODULE] = clock
    try:
        importlib.reload(inference)
    finally:
        restore(saved)


def restore(saved: dict[str, ModuleType | None]) -> None:
    for name, module in saved.items():
        if module is None:
            sys.modules.pop(name)
        else:
            sys.modules[name] = module
