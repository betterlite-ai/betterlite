from typing import Literal, cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock_mantle.chat.transformation import BedrockMantleChatConfig
from pydantic import JsonValue

from .models import CHAT_PAYLOAD, ChatCase
from .support import JSON, environment
from .views import ChatApi


class ReasoningRecorder:
    def __init__(self, answer: Literal["supported", "unsupported", "panic"]) -> None:
        self.answer = answer
        self.calls: list[JsonValue] = []

    def __call__(self, model: str, custom_llm_provider: str | None = None) -> bool:
        self.calls.append([model, custom_llm_provider])
        if self.answer == "panic":
            raise RuntimeError("fixture reasoning probe")
        return self.answer == "supported"


def chat_result(config: ChatApi, case: ChatCase, recorder: ReasoningRecorder) -> JsonValue:
    project_id = case.get("project_id")
    litellm_params: dict[str, JsonValue] = {}
    if project_id is not None:
        litellm_params["aws_bedrock_project_id"] = project_id
    headers = case.get("headers")
    merged = config.validate_environment(
        headers={} if headers is None else dict(headers),
        model=case["model"],
        messages=[],
        optional_params={},
        litellm_params=litellm_params,
        api_key=case.get("api_key"),
    )
    params = config.get_supported_openai_params(case["model"])
    handler = config.get_model_response_iterator(
        iter(()), sync_stream=case.get("sync_stream") is True, json_mode=case.get("json_mode")
    )
    error = case["error"]
    given = error.get("headers")
    failure = config.get_error_class(
        error["message"], error["status_code"], {} if given is None else given
    )
    returned = failure.headers
    return {
        "headers": merged,
        "params": JSON.validate_python(params),
        "requests": recorder.calls,
        "error": {
            "status": failure.status_code,
            "message": failure.message,
            "headers": {} if returned is None else dict(returned),
        },
        "json_mode": handler.json_mode is True,
    }


def chat_cases(payload: JsonValue) -> JsonValue:
    parsed = CHAT_PAYLOAD.validate_python(payload)
    catalog = parsed.get("catalog")
    chat_models: list[str] = []
    text_models: list[str] = []
    if catalog is not None:
        listed = catalog.get("chat_models")
        if listed is not None:
            chat_models = listed
        textual = catalog.get("text_models")
        if textual is not None:
            text_models = textual
    config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L36-L143
        "ChatApi", BedrockMantleChatConfig()
    )
    results: list[JsonValue] = []
    for case in parsed["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        answer = case.get("reasoning")
        recorder = ReasoningRecorder("unsupported" if answer is None else answer)
        with (
            environment(values),
            patch.object(litellm, "open_ai_chat_completion_models", chat_models),
            patch.object(litellm, "open_ai_text_completion_models", text_models),
            patch.object(litellm, "supports_reasoning", recorder),
        ):
            results.append(chat_result(config, case, recorder))
    return {"results": results}
