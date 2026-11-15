from collections.abc import Iterator
from datetime import UTC, datetime
from typing import Final, cast

import httpx
import litellm
from litellm import utils
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.completion.transformation import AnthropicTextConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .views import (
    AuthCase,
    BodyView,
    ErrorCase,
    Input,
    OptionsInput,
    PromptCase,
    Registry,
    ReplyCase,
    ResponseView,
    TextConfig,
    Turn,
)

litellm.suppress_debug_info = True

MODEL: Final = "claude-2"
# a reply made now carries a Unix time far past this, and ours carries the fixed clock
CREATED_FLOOR: Final = 1_000_000_000
NO_STREAM: Final[Iterator[str]] = iter(())
BODY: Final[TypeAdapter[BodyView]] = TypeAdapter(BodyView)
RESPONSE: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
REGISTRY: Final[Registry] = cast(Registry, utils)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7258-L7320


def as_config(config: AnthropicTextConfig) -> TextConfig:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/completion/transformation.py#L53-L79
        TextConfig,
        config,
    )


def selected(model: str) -> bool:
    _, provider, _, _ = get_llm_provider(model=model, custom_llm_provider="anthropic")
    return provider == "anthropic_text"


def request_body(
    config: TextConfig, model: str, turns: list[Turn], params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    return config.transform_request(
        model=model,
        messages=turns,
        optional_params=params,
        litellm_params={},
        headers={},
    )


def prompt_case(config: TextConfig, case: PromptCase) -> str:
    entry = case["custom_prompt"]
    if entry is not None:
        REGISTRY.register_prompt_template(
            case["model"],
            entry["roles"],
            entry["initial_prompt_value"],
            entry["final_prompt_value"],
        )
    body = request_body(config, case["model"], case["messages"], {})
    return BODY.validate_python(body)["prompt"]


def auth_case(config: TextConfig, case: AuthCase) -> JsonValue:
    try:
        headers = config.validate_environment(
            headers=dict(case["headers"]),
            model=MODEL,
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=case["api_key"],
        )
    except ValueError as error:
        return {"ok": False, "message": str(error)}
    return {"ok": True, "headers": JSON.validate_python(headers)}


def options(input_value: OptionsInput) -> JsonValue:
    constructor = input_value["constructor"]
    config = as_config(
        AnthropicTextConfig(
            max_tokens_to_sample=constructor["max_tokens_to_sample"],
            stop_sequences=constructor["stop_sequences"],
            top_k=constructor["top_k"],
            metadata=constructor["metadata"],
        )
    )
    cases: list[JsonValue] = []
    for case in input_value["cases"]:
        mapped = config.map_openai_params(
            case["non_default"], dict(case["existing"]), case["model"], drop_params=False
        )
        # transform_request fills the configured defaults into the dict it gets, so it gets a copy
        body = request_body(config, case["model"], case["messages"], dict(mapped))
        cases.append({"mapped": mapped, "body": body})
    supported = JSON.validate_python(config.get_supported_openai_params(model=MODEL))
    return {"supported": supported, "cases": cases}


def reply_case(config: TextConfig, case: ReplyCase) -> JsonValue:
    request = httpx.Request("POST", "https://api.anthropic.com/v1/complete")
    raw = httpx.Response(case["status"], content=case["body"].encode(), request=request)
    initial = ModelResponse(id="chatcmpl-kept", model="unset")
    initial.choices[0].message.content = case["initial_content"]
    logging_obj = Logging(
        model=MODEL,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2024, 1, 2, tzinfo=UTC),
        litellm_call_id="call",
        function_id="function",
    )
    try:
        result = config.transform_response(
            model=case["model"],
            raw_response=raw,
            model_response=initial,
            logging_obj=logging_obj,
            request_data={},
            messages=case["messages"],
            optional_params={},
            litellm_params={},
            encoding=None,
        )
    except BaseLLMException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L65
        return {"ok": False, "status": error.status_code, "message": error.message}
    view = RESPONSE.validate_json(result.model_dump_json())
    choice = view["choices"][0]
    usage = view["usage"]
    return {
        "ok": True,
        "content": choice["message"]["content"],
        "finish_reason": choice["finish_reason"],
        "id": view["id"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
        "usage": [usage["prompt_tokens"], usage["completion_tokens"], usage["total_tokens"]],
    }


def error_case(config: TextConfig, case: ErrorCase) -> JsonValue:
    error = config.get_error_class(case["message"], case["status_code"], case["headers"])
    return {
        "status_code": error.status_code,
        "message": error.message,
        "method": error.request.method,
        "url": str(error.request.url),
        "response_status": error.response.status_code,
        "headers_kept": error.headers is not None,
    }


def run(input_value: Input) -> JsonValue:
    config = as_config(AnthropicTextConfig())
    output: JsonValue
    if input_value["function"] == "select":
        flags: list[JsonValue] = [selected(model) for model in input_value["models"]]
        output = {"selected": flags}
    elif input_value["function"] == "prompt":
        prompts: list[JsonValue] = [prompt_case(config, case) for case in input_value["cases"]]
        output = {"prompts": prompts}
    elif input_value["function"] == "auth":
        results: list[JsonValue] = [auth_case(config, case) for case in input_value["cases"]]
        output = {"results": results}
    elif input_value["function"] == "options":
        output = options(input_value)
    elif input_value["function"] == "reply":
        replies: list[JsonValue] = [reply_case(config, case) for case in input_value["cases"]]
        output = {"replies": replies}
    elif input_value["function"] == "chunk":
        parser = config.get_model_response_iterator(NO_STREAM, sync_stream=True, json_mode=False)
        parsed = [parser.chunk_parser(chunk) for chunk in input_value["chunks"]]
        chunks: list[JsonValue] = [
            {
                "text": chunk["text"],
                "is_finished": chunk["is_finished"],
                "finish_reason": chunk["finish_reason"],
                "index": chunk["index"],
            }
            for chunk in parsed
        ]
        output = {"chunks": chunks}
    else:
        errors: list[JsonValue] = [error_case(config, case) for case in input_value["cases"]]
        output = {"errors": errors}
    return JSON.validate_python(output)
