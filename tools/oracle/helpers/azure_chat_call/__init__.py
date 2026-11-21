import asyncio
from typing import cast

import litellm
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.openai.openai import OpenAIChatCompletion
from openai import OpenAIError
from pydantic import JsonValue

from .clients import (
    Capture,
    ScriptedAzure,
    ScriptedChat,
    ScriptedOSeries,
    completion_kwargs,
    o_series_kwargs,
)
from .fixtures import Fixture, case_answer, caller_of, client_for, supplied_client
from .types import (
    AsyncChat,
    CallCase,
    CompletionKwargs,
    Input,
    OSeriesChat,
    Outcome,
    SyncChat,
)
from .views import Recorder, azure_error_view, reply_view, sdk_error_view, stored_headers


def report(
    handler: ScriptedAzure, fixture: Fixture, recorder: Recorder, result: Outcome
) -> JsonValue:
    return {
        "outcome": result.outcome,
        "gets": handler.gets,
        "gateways": handler.gateways,
        "requests": fixture.requests,
        "pre": recorder.pre,
        "post": recorder.post,
        "response_headers": stored_headers(recorder),
        "reply": result.reply,
        "error": result.error,
        "forward": result.forward,
    }


async def cancelled_call(
    handler: ScriptedChat, kwargs: CompletionKwargs, fixture: Fixture
) -> None:
    task = asyncio.create_task(cast(AsyncChat, handler).completion(**kwargs))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L577-L652
    await fixture.started.wait()
    task.cancel()
    try:
        await task
    except asyncio.CancelledError:
        return


async def completion_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case), caller_of(case))
    mock = client_for(case, fixture)
    handler = ScriptedChat(mock)
    recorder = Recorder()
    kwargs = completion_kwargs(case, recorder, supplied_client(case, mock))
    if case.get("cancel"):
        await cancelled_call(handler, kwargs, fixture)
        return report(handler, fixture, recorder, Outcome(outcome="cancelled"))
    try:
        if kwargs["acompletion"]:
            result = await cast(AsyncChat, handler).completion(**kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L577-L652
        else:
            result = cast(SyncChat, handler).completion(**kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L500-L575
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        failure = Outcome(outcome="error", error=azure_error_view(error))
        return report(handler, fixture, recorder, failure)
    except OpenAIError as error:
        failure = Outcome(outcome="error", error=sdk_error_view(error))
        return report(handler, fixture, recorder, failure)
    buffered = Outcome(outcome="buffered", reply=reply_view(result))
    return report(handler, fixture, recorder, buffered)


async def o_series_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case), caller_of(case))
    handler = ScriptedOSeries(client_for(case, fixture))
    recorder = Recorder()
    kwargs = o_series_kwargs(case, recorder, supplied_client(case, handler.mock))
    capture = Capture()
    original = OpenAIChatCompletion.completion  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    OpenAIChatCompletion.completion = capture  # type: ignore[method-assign, assignment]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    try:
        cast(OSeriesChat, handler).completion(**kwargs)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/chat/o_series_handler.py#L23-L79
    finally:
        OpenAIChatCompletion.completion = original  # type: ignore[method-assign]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L645-L646
    return report(
        handler,
        fixture,
        recorder,
        Outcome(outcome="delegated", forward=capture.calls[0]),
    )


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    if input_value["function"] == "completion":
        return {"calls": [asyncio.run(completion_call(case)) for case in input_value["cases"]]}
    return {"calls": [asyncio.run(o_series_call(case)) for case in input_value["cases"]]}
