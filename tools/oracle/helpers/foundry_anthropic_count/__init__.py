import asyncio
import logging
import os
from collections.abc import Awaitable
from typing import Final, TypeVar

import litellm
from litellm.constants import request_timeout as default_timeout
from litellm.llms.anthropic.common_utils import AnthropicError
from litellm.llms.azure_ai.anthropic.count_tokens.handler import (
    AzureAIAnthropicCountTokensHandler,
)
from litellm.llms.azure_ai.anthropic.count_tokens.token_counter import (
    AzureAIAnthropicTokenCounter,
)
from litellm.llms.azure_ai.anthropic.count_tokens.transformation import (
    AzureAIAnthropicCountTokensConfig,
)
from litellm.types.utils import TokenCountResponse
from pydantic import JsonValue as Json

from foundry_anthropic_count.records import (
    DECODER,
    CounterCase,
    EndpointCall,
    Env,
    HandleCase,
    Input,
    Params,
)
from foundry_anthropic_count.transport import Sent, Warnings, install

INTERNAL_SERVER_ERROR: Final = 500
PROCESSING_PREFIX: Final = "CountTokens processing error: "
Outcome = TypeVar("Outcome")


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return "processing" if message.startswith(PROCESSING_PREFIX) else "generic"
    return message


def result_json(result: TokenCountResponse) -> Json:
    result.error_message = error_message_kind(result.error_message, result.status_code)
    return DECODER.validate_json(result.model_dump_json())


def use_environment(values: Env) -> None:
    os.environ.clear()
    os.environ.update(values)


def prepare(case: HandleCase | CounterCase) -> None:
    environment = case.get("environment")
    use_environment({} if environment is None else environment)
    overrides = case.get("globals")
    litellm.api_key = None if overrides is None else overrides.get("api_key")
    litellm.azure_key = None if overrides is None else overrides.get("azure_key")
    litellm.enable_azure_ad_token_refresh = False
    seconds = case.get("request_timeout")
    litellm.request_timeout = default_timeout if seconds is None else seconds  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L89  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L89


async def answered(sent: Sent, response: Awaitable[Outcome]) -> Outcome:
    install(sent)
    return await response


async def handled(
    sent: Sent, handler: AzureAIAnthropicCountTokensHandler, case: HandleCase
) -> Json:
    install(sent)
    return DECODER.validate_python(await handler.handle_count_tokens_request(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/anthropic/count_tokens/handler.py#L27-L127
        model=case["model"],
        messages=case["messages"],
        api_key=case["api_key"],
        api_base=case["api_base"],
        litellm_params=case.get("litellm_params"),
        timeout=case.get("timeout"),
        tools=case.get("tools"),
        system=case.get("system"),
    ))


def endpoint(call: EndpointCall) -> Json:
    config = AzureAIAnthropicCountTokensConfig()
    urls: list[Json] = [config.get_count_tokens_endpoint(base) for base in call["api_bases"]]
    return {"urls": urls}


def handle_case(case: HandleCase) -> Json:
    prepare(case)
    sent = Sent(case.get("reply"))
    handler = AzureAIAnthropicCountTokensHandler()
    try:
        response: Json = {"value": asyncio.run(handled(sent, handler, case))}
    except AnthropicError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L309
        response = {
            "error": {
                "status_code": error.status_code,
                "processing": error.message.startswith(PROCESSING_PREFIX),
            }
        }
    return {"request": sent.first, "response": response}


def counter_case(case: CounterCase) -> Json:
    prepare(case)
    sent = Sent(case.get("reply"))
    counter = AzureAIAnthropicTokenCounter()
    selected = counter.should_use_token_counting_api(case.get("provider"))
    params = case.get("litellm_params")
    deployment: Params | None = None if params is None else {"litellm_params": params}
    request_model = case.get("request_model")
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.handlers = [logging.NullHandler()]
    logger.setLevel(logging.DEBUG)
    logger.filters.insert(0, warnings)
    try:
        result = asyncio.run(
            answered(
                sent,
                counter.count_tokens(
                    model_to_use=case["model"],
                    messages=case.get("messages"),
                    contents=case.get("contents"),
                    deployment=deployment,
                    request_model="" if request_model is None else request_model,
                    tools=case.get("tools"),
                    system=case.get("system"),
                ),
            )
        )
    finally:
        logger.filters.remove(warnings)
    return {
        "selected": selected,
        "result": None if result is None else result_json(result),
        "request": sent.first,
        "warnings": warnings.entries,
    }


def run(argument: Input) -> Json:
    match argument["function"]:
        case "endpoint":
            return endpoint(argument)
        case "handle":
            answers: list[Json] = [handle_case(case) for case in argument["cases"]]
            return {"calls": answers}
        case "counter":
            counted: list[Json] = [counter_case(case) for case in argument["cases"]]
            return {"calls": counted}
