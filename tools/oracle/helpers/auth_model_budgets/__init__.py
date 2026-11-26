import asyncio

import litellm
from pydantic import JsonValue, TypeAdapter

from .case import key_fallback_call, team_budget_call, user_budget_call
from .end_user import end_user_apply_call, end_user_update_call
from .spend import global_spend_call
from .wire import Call, Request

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


async def call_answer(call: Call) -> JsonValue:
    try:
        return await answer_for(call)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L417-L488
        return {"ok": False}


async def answer_for(call: Call) -> JsonValue:
    match call["function"]:
        case "key_fallback":
            return {"ok": True, "value": await key_fallback_call(call)}
        case "user_budget":
            return {"ok": True, "value": await user_budget_call(call)}
        case "team_budget":
            return {"ok": True, "value": await team_budget_call(call)}
        case "end_user_apply":
            return {"ok": True, "value": end_user_apply_call(call)}
        case "end_user_update":
            return {"ok": True, "value": end_user_update_call(call)}
        case "global_spend":
            return {"ok": True, "value": await global_spend_call(call)}


async def answer(parsed: Request) -> JsonValue:  # ast-grep-ignore: no-one-line-functions
    return [await call_answer(call) for call in parsed["calls"]]


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = TypeAdapter(Request).validate_python(input_value)
    return asyncio.run(answer(parsed))
