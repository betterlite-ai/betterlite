import asyncio
from typing import Literal, Protocol, TypedDict

import httpx
from litellm.caching.dual_cache import DualCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.lakera_ai import lakeraAI_Moderation
from litellm.types.guardrails import LakeraCategoryThresholds
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException

DETAIL: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    function: Literal["lakera_moderation_threshold_check"]
    category_thresholds: LakeraCategoryThresholds | None
    data: dict[str, JsonValue]
    response: dict[str, JsonValue]


# The guardrail's own hook types its return as a bare dict, so this names what the oracle reads
class Hook(Protocol):
    async def async_pre_call_hook(
        self,
        user_api_key_dict: UserAPIKeyAuth,
        cache: DualCache,
        data: dict[str, JsonValue],
        call_type: Literal["completion"],
    ) -> None | str | Exception | dict[str, JsonValue]: ...


def as_hook(guardrail: lakeraAI_Moderation) -> Hook:
    hook: Hook = guardrail
    return hook


class Recorder(httpx.MockTransport):
    def __init__(self, reply: dict[str, JsonValue]) -> None:
        super().__init__(self.handle)
        self.reply = reply

    def handle(self, _request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json=self.reply)


def run(input_value: Input) -> JsonValue:
    guardrail = lakeraAI_Moderation(
        moderation_check="pre_call",
        category_thresholds=input_value["category_thresholds"],
        api_base="http://lakera.test",
        api_key="test",
    )
    handler = AsyncHTTPHandler()
    recorder = Recorder(input_value["response"])
    handler.client = httpx.AsyncClient(transport=recorder)
    guardrail.async_handler = handler
    hook = as_hook(guardrail)
    verdict: JsonValue = None
    try:
        _answer = asyncio.run(
            hook.async_pre_call_hook(
                UserAPIKeyAuth(), DualCache(), input_value["data"], "completion"
            )
        )
    except HTTPException as error:
        detail = DETAIL.validate_python(error.detail)
        rejection: JsonValue = {"status": error.status_code, "detail": detail}
        verdict = rejection
    return {"rejected": verdict}
