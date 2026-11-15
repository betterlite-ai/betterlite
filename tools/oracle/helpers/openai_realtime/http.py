import litellm
from litellm.llms.openai.realtime.http_transformation import OpenAIRealtimeHTTPConfig
from pydantic import JsonValue

from .types import HEADERS, JSON, HttpSetup, Input, RealtimeHttpConfig

CONFIG: RealtimeHttpConfig = OpenAIRealtimeHTTPConfig()


def http_view(model: str, setup: HttpSetup) -> JsonValue:
    base = CONFIG.get_api_base(setup["api_base"])
    key = CONFIG.get_api_key(setup["api_key"])
    headers = HEADERS.validate_python(
        CONFIG.validate_environment(dict(setup["headers"]), model, key)
    )
    payload: dict[str, JsonValue] = {
        "api_base": base,
        "api_key": key,
        "complete_url": CONFIG.get_complete_url(base, model, setup["api_version"]),
        "calls_url": CONFIG.get_realtime_calls_url(base, model, setup["api_version"]),
        "transcription_url": CONFIG.get_transcription_session_url(
            base, model, setup["api_version"]
        ),
        "headers": headers,
    }
    return JSON.validate_python(payload)


def http_setup(model: str, setup: HttpSetup) -> JsonValue:
    saved = (litellm.api_base, litellm.api_key, litellm.openai_key)
    litellm.api_base, litellm.api_key, litellm.openai_key = (
        setup["litellm_api_base"],
        setup["litellm_api_key"],
        setup["litellm_openai_key"],
    )
    try:
        return http_view(model, setup)
    finally:
        litellm.api_base, litellm.api_key, litellm.openai_key = saved


def http_answer(input_value: Input) -> JsonValue:
    http = [http_setup(input_value["model"], setup) for setup in input_value["http_setups"]]
    return {"http": http}
