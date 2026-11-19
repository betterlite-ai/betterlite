import asyncio

import litellm
from pydantic import JsonValue

from b5368 import stubs
from b5368.stubs import (
    ASYNC_CLIENT,
    RECORDER,
    SYNC_CLIENT,
    Answer,
    ErrorReport,
    Input,
    install_state,
)

litellm.suppress_debug_info = True


def prepare(input_value: Input) -> dict[str, JsonValue]:
    install_state(input_value)
    RECORDER.reset(input_value)
    return dict(input_value["request"])


def answer(result: JsonValue, error: Exception | None) -> Answer:
    calls = RECORDER.calls
    seen = RECORDER.seen
    report: ErrorReport | None = None
    if error is not None:
        report = {"error_class": type(error).__name__}
    return {
        "result": stubs.result_value(result),
        "error": report,
        "calls": calls,
        "seen": seen,
    }


def run_sync(input_value: Input) -> Answer:
    request = prepare(input_value)
    wrapper = SYNC_CLIENT(stubs.completion)
    result: JsonValue = None
    error: Exception | None = None
    try:
        result = wrapper(**request)
    except Exception as caught:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1899-L1930
        error = caught
    return answer(result, error)


async def run_async(input_value: Input) -> Answer:
    request = prepare(input_value)
    wrapper = ASYNC_CLIENT(stubs.acompletion)
    result: JsonValue = None
    error: Exception | None = None
    try:
        result = await wrapper(**request)
    except Exception as caught:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L2196-L2315
        error = caught
    return answer(result, error)


def run(input_value: Input) -> Answer:
    mode = input_value.get("mode")
    if mode == "async":
        return asyncio.run(run_async(input_value))
    return run_sync(input_value)
