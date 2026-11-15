from typing import TypedDict

from pydantic import JsonValue, TypeAdapter

from .boundary import RESPONSES
from .environment import install_environment


class ResponsesCase(TypedDict):
    api_base: str | None
    environment_base: str | None


def responses_value(case: ResponsesCase) -> JsonValue:
    install_environment(base=case["environment_base"], key=None, forced=None, openai_key=None)
    try:
        url = RESPONSES.get_complete_url(case["api_base"], {})
    except ValueError:
        return {"ok": False}
    return {
        "ok": True,
        "url": url,
        "provider": RESPONSES.custom_llm_provider.value,
        "websocket": RESPONSES.supports_native_websocket(),
    }


def responses_cases(params: JsonValue) -> JsonValue:
    cases = TypeAdapter(list[ResponsesCase]).validate_python(params)
    return {"results": [responses_value(case) for case in cases]}
