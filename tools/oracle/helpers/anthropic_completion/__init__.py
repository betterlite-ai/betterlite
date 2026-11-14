from collections.abc import Iterator
from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.completion.transformation import AnthropicTextConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.types.utils import Choices, Message, ModelResponse
from litellm.utils import register_prompt_template
from pydantic import JsonValue, TypeAdapter

from anthropic_completion.shapes import (
    Accepted,
    AuthCase,
    ErrorCase,
    ErrorView,
    Failed,
    Input,
    OptionCase,
    OptionsInput,
    OptionsOutput,
    OptionView,
    Output,
    PromptCase,
    Refused,
    Replied,
    ReplyCase,
    Turn,
)
from anthropic_completion.views import BodyView, Messages, ResponseView, TextConfig

COMPLETE_URL: Final = "https://api.anthropic.com/v1/complete"
MODEL: Final = "claude-2"
# a reply made now carries a Unix time far past this, and ours carries the fixed clock
CREATED_FLOOR: Final = 1_000_000_000
NO_STREAM: Final[Iterator[str]] = iter(())
BODY: Final[TypeAdapter[BodyView]] = TypeAdapter(BodyView)
RESPONSE: Final[TypeAdapter[ResponseView]] = TypeAdapter(ResponseView)


def selected(model: str) -> bool:
    _, provider, _, _ = get_llm_provider(model=model, custom_llm_provider="anthropic")
    return provider == "anthropic_text"


def request_body(
    config: TextConfig, model: str, turns: list[Turn], params: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    return config.transform_request(
        model=model,
        messages=TypeAdapter(Messages).validate_python(turns),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=params,
        litellm_params={},
        headers={},
    )


def prompt_case(config: TextConfig, case: PromptCase) -> str:
    entry = case["custom_prompt"]
    if entry is not None:
        register_prompt_template(
            model=case["model"],
            roles=entry["roles"],
            initial_prompt_value=entry["initial_prompt_value"],
            final_prompt_value=entry["final_prompt_value"],
        )
    body = request_body(config, case["model"], case["messages"], {})
    return BODY.validate_python(body)["prompt"]


def auth_case(config: TextConfig, case: AuthCase) -> Accepted | Refused:
    try:
        headers = config.validate_environment(
            headers=dict(case["headers"]),
            model=MODEL,
            messages=TypeAdapter(Messages).validate_python([]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params={},
            litellm_params={},
            api_key=case["api_key"],
            api_base=None,
        )
    except ValueError as error:
        return {"ok": False, "message": str(error)}
    return {"ok": True, "headers": headers}


def option_case(config: TextConfig, case: OptionCase) -> OptionView:
    mapped = config.map_openai_params(
        non_default_params=case["non_default"],
        optional_params=dict(case["existing"]),
        model=case["model"],
        drop_params=False,
    )
    # transform_request fills the configured defaults into the dict it gets, so it gets a copy
    body = request_body(config, case["model"], case["messages"], dict(mapped))
    return {"mapped": mapped, "body": body}


def options(input_value: OptionsInput) -> OptionsOutput:
    constructor = input_value["constructor"]
    config: TextConfig = AnthropicTextConfig(
        max_tokens_to_sample=constructor["max_tokens_to_sample"],
        stop_sequences=constructor["stop_sequences"],
        top_k=constructor["top_k"],
        metadata=constructor["metadata"],
    )
    cases = input_value["cases"]
    return {
        "supported": config.get_supported_openai_params(model=MODEL),
        "cases": [option_case(config, case) for case in cases],
    }


def reply_case(config: TextConfig, case: ReplyCase) -> Replied | Failed:
    turns = case["messages"]
    raw = httpx.Response(
        case["status"],
        content=case["body"].encode(),
        request=httpx.Request("POST", COMPLETE_URL),
    )
    initial = Choices(message=Message(role="assistant", content=case["initial_content"]))
    logging_obj = Logging(
        model=MODEL,
        messages=[],
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
            model_response=ModelResponse(id="chatcmpl-kept", choices=[initial], model="unset"),
            logging_obj=logging_obj,
            request_data={},
            messages=TypeAdapter(Messages).validate_python(turns),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params={},
            litellm_params={},
            encoding=None,
            api_key=None,
            json_mode=None,
        )
    except BaseLLMException as error:
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


def error_case(config: TextConfig, case: ErrorCase) -> ErrorView:
    error = config.get_error_class(
        error_message=case["message"],
        status_code=case["status_code"],
        headers=case["headers"],
    )
    return {
        "status_code": error.status_code,
        "message": error.message,
        "method": error.request.method,
        "url": str(error.request.url),
        "response_status": error.response.status_code,
        "headers_kept": error.headers is not None,
    }


def run(input_value: Input) -> Output:
    config: TextConfig = AnthropicTextConfig()
    output: Output
    if input_value["function"] == "select":
        output = {"selected": [selected(model) for model in input_value["models"]]}
    elif input_value["function"] == "prompt":
        output = {"prompts": [prompt_case(config, case) for case in input_value["cases"]]}
    elif input_value["function"] == "auth":
        output = {"results": [auth_case(config, case) for case in input_value["cases"]]}
    elif input_value["function"] == "options":
        output = options(input_value)
    elif input_value["function"] == "reply":
        output = {"replies": [reply_case(config, case) for case in input_value["cases"]]}
    elif input_value["function"] == "chunk":
        parser = config.get_model_response_iterator(NO_STREAM, sync_stream=True, json_mode=False)
        parsed = [parser.chunk_parser(chunk) for chunk in input_value["chunks"]]
        output = {
            "chunks": [
                {
                    "text": chunk["text"],
                    "is_finished": chunk["is_finished"],
                    "finish_reason": chunk["finish_reason"],
                    "index": chunk["index"],
                }
                for chunk in parsed
            ]
        }
    else:
        output = {"errors": [error_case(config, case) for case in input_value["cases"]]}
    return output
