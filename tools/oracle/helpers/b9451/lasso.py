from typing import Literal, Protocol, TypedDict

from litellm import DualCache
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.lasso.lasso import LassoGuardrail
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import LLMResponseTypes
from pydantic import JsonValue as Json

from .scripted import Answer, PreCallHooks, attempt, outcome_view, post_call_contents, scripted

HEADER_NAMES = ("content-type", "lasso-api-key", "lasso-conversation-id", "lasso-user-id")


class LassoCase(TypedDict):
    vendor: Literal["lasso"]
    hook: Literal["pre_call", "moderation", "post_call"]
    gate: bool
    api_key: str
    conversation_id: str
    mask: bool
    timeout: float
    data: dict[str, Json]
    outputs: list[str]
    answers: list[Answer]


class LassoHooks(PreCallHooks, Protocol):
    async def async_moderation_hook(
        self, data: dict[str, Json], user: UserAPIKeyAuth, call_type: Literal["completion"], cache: DualCache, /
    ) -> None: ...

    async def async_post_call_success_hook(
        self, data: dict[str, Json], user: UserAPIKeyAuth, response: LLMResponseTypes, /
    ) -> LLMResponseTypes: ...


def event_hook(case: LassoCase) -> GuardrailEventHooks:
    match case["hook"]:
        case "pre_call":
            return GuardrailEventHooks.pre_call if case["gate"] else GuardrailEventHooks.post_call
        case "moderation":
            return GuardrailEventHooks.during_call if case["gate"] else GuardrailEventHooks.pre_call
        case "post_call":
            return GuardrailEventHooks.post_call if case["gate"] else GuardrailEventHooks.pre_call


async def call_hook(guardrail: LassoHooks, case: LassoCase) -> dict[str, Json]:
    user = UserAPIKeyAuth(api_key="lasso-oracle-key")
    data = case["data"]
    match case["hook"]:
        case "pre_call":
            await guardrail.async_pre_call_hook(user, DualCache(), data, "completion")
        case "moderation":
            await guardrail.async_moderation_hook(data, user, "completion", DualCache())
        case "post_call":
            return await post_call_contents(guardrail.async_post_call_success_hook, data, user, case["outputs"])
    return {"data": data}


async def run_lasso(case: LassoCase) -> Json:
    async with scripted(case["answers"]) as server:
        guardrail: LassoHooks = LassoGuardrail(
            api_key=case["api_key"],
            api_base=server.base_url,
            conversation_id=case["conversation_id"],
            mask=case["mask"],
            timeout=case["timeout"],
            guardrail_name="lasso",
            event_hook=event_hook(case),
            default_on=True,
        )
        outcome = await attempt(call_hook(guardrail, case))
        return outcome_view(server, outcome, HEADER_NAMES, {})
