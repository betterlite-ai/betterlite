import asyncio

from pydantic import JsonValue

from .calls import enforce_frame, refund, reserve_io, reserve_tpm
from .fakes import RecordingCache
from .limiter import make_limiter
from .types import Input


async def handle(input_value: Input) -> JsonValue:
    cache = RecordingCache(input_value["cache"])
    limiter = make_limiter(cache, input_value["now"], input_value["groups"])
    if input_value["function"] == "reserve_tpm_tokens":
        return await reserve_tpm(limiter, cache, input_value)
    if input_value["function"] == "refund_reserved_tokens":
        return await refund(limiter, cache, input_value)
    if input_value["function"] == "reserve_io_tokens":
        return await reserve_io(limiter, cache, input_value)
    return await enforce_frame(limiter, cache, input_value)


def run(input_value: Input) -> JsonValue:
    with asyncio.Runner() as runner:
        return runner.run(handle(input_value))
