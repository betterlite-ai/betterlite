import asyncio
from collections.abc import AsyncIterator, Awaitable
from typing import Final, cast

from litellm.llms.azure_ai.agents.handler import AzureAIAgentsHandler
from litellm.llms.azure_ai.agents.transformation import (
    AzureAIAgentsConfig,
    AzureAIAgentsError,
)
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue

from .fakes import (
    AsyncClient,
    Clock,
    Factory,
    SyncClient,
    install_fixtures,
)
from .types import (
    JSON,
    REPLY_DUMP,
    ConfigProduct,
    DispatchCall,
    JsonObject,
    RunCall,
)

TIMEOUT: Final = 60.0


def reply_view(response: ModelResponse) -> JsonObject:
    dumped = REPLY_DUMP.validate_json(response.model_dump_json())
    first = dumped["choices"][0]
    message = first["message"]
    hidden = cast(JsonObject, response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L226-L236
    usage = dumped.get("usage")
    counted: JsonValue = None
    if usage is not None:
        counted = {
            "prompt_tokens": usage.get("prompt_tokens"),
            "completion_tokens": usage.get("completion_tokens"),
            "total_tokens": usage.get("total_tokens"),
        }
    block: JsonObject = {
        "role": message.get("role"),
        "content": message.get("content"),
        "annotations": message.get("annotations"),
    }
    choice: JsonObject = {
        "index": first["index"],
        "finish_reason": first["finish_reason"],
        "message": block,
    }
    choices: list[JsonValue] = [choice]
    hidden_params: JsonObject = {"thread_id": hidden.get("thread_id")}
    reply: JsonObject = {
        "model": dumped["model"],
        "choices": choices,
        "hidden_params": hidden_params,
        "usage": counted,
    }
    return reply


def success(response: ModelResponse) -> JsonObject:
    reply = reply_view(response)
    return {"ok": True, "reply": reply}


def failure(error: AzureAIAgentsError) -> JsonObject:
    status = error.status_code
    return {"ok": False, "status": status, "message": str(error)}


def sync_run(call: RunCall, factory: Factory) -> JsonValue:
    handler = AzureAIAgentsHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L122-L123
    client = SyncClient(factory.recorder) if call["supplied"] else None
    try:
        response = handler.completion(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L302-L350
            model=call["model"],
            messages=cast(list[dict[str, object]], call["messages"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L305
            api_base=call["api_base"],
            api_key=call["api_key"],
            model_response=ModelResponse(),
            logging_obj=None,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L309
            optional_params=call["optional_params"],
            litellm_params=call["litellm_params"],
            timeout=TIMEOUT,
            client=client,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L313
            headers=call["headers"],
        )
    except AzureAIAgentsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L48-L49
        return failure(error)
    return success(response)


async def async_run(call: RunCall, factory: Factory) -> JsonValue:
    handler = AzureAIAgentsHandler()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L122-L123
    client = AsyncClient(factory.recorder) if call["supplied"] else None
    try:
        response = await handler.acompletion(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L426-L478
            model=call["model"],
            messages=cast(list[dict[str, object]], call["messages"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L429
            api_base=call["api_base"],
            api_key=call["api_key"],
            model_response=ModelResponse(),
            logging_obj=None,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L433
            optional_params=call["optional_params"],
            litellm_params=call["litellm_params"],
            timeout=TIMEOUT,
            client=client,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L437
            headers=call["headers"],
        )
    except AzureAIAgentsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L48-L49
        return failure(error)
    return success(response)


def run_answer(call: RunCall) -> JsonValue:
    clock = Clock()
    factory = Factory(call["steps"])
    install_fixtures(clock, factory)
    if call["mode"] == "async":
        outcome = asyncio.run(async_run(call, factory))
    else:
        outcome = sync_run(call, factory)
    client = "supplied" if call["supplied"] else factory.kind
    return {
        "client": client,
        "ssl_verify": factory.ssl_verify,
        "requests": JSON.validate_python(factory.recorder.requests),
        "sleeps": JSON.validate_python(clock.waits),
        "outcome": outcome,
    }


def completion_answer(
    factory: Factory, clock: Clock, response: ModelResponse
) -> JsonObject:
    return {
        "ok": True,
        "route": "completion",
        "client": factory.kind,
        "ssl_verify": factory.ssl_verify,
        "requests": JSON.validate_python(factory.recorder.requests),
        "sleeps": JSON.validate_python(clock.waits),
        "outcome": success(response),
    }


def config_call(call: DispatchCall) -> ConfigProduct:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L310-L397
        ConfigProduct,
        AzureAIAgentsConfig.completion(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L310-L397
            model=call["model"],
            messages=call["messages"],
            api_base=call["api_base"],
            api_key=call["api_key"],
            model_response=ModelResponse(),
            logging_obj=None,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L316
            optional_params=call["optional_params"],
            litellm_params=call["litellm_params"],
            timeout=TIMEOUT,
            acompletion=call["acompletion"],
            stream=call["stream"],
            headers=call["headers"],
        ),
    )


async def config_answer(
    call: DispatchCall, factory: Factory, clock: Clock
) -> JsonObject:
    try:
        if call["acompletion"] and call["stream"]:
            stream = cast(AsyncIterator[ModelResponseStream], config_call(call))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L310-L397
            async for _chunk in stream:
                pass
            request = factory.recorder.requests[0]
            return {
                "ok": True,
                "route": "stream",
                "url": request["url"],
                "headers": request["headers"],
                "body": request["body"],
                "ssl_verify": factory.ssl_verify,
            }
        if call["acompletion"]:
            response = await cast(Awaitable[ModelResponse], config_call(call))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L310-L397
            return completion_answer(factory, clock, response)
        response = cast(ModelResponse, config_call(call))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L310-L397
    except AzureAIAgentsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/transformation.py#L48-L49
        return failure(error)
    except ValueError as error:
        return {"ok": False, "status": None, "message": str(error)}
    return completion_answer(factory, clock, response)
