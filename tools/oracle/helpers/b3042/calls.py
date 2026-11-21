import asyncio

from litellm.proxy._types import UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

from .answers import answer, raised_json, response_json
from .fakes import RecordingCache
from .types import FrameInput, Operations, RefundInput, ReserveIoInput, ReserveTpmInput


async def reserve_tpm(
    limiter: Operations, cache: RecordingCache, call: ReserveTpmInput
) -> JsonValue:
    response = await limiter.reserve_tpm_tokens(call["descriptors"], call["estimated_tokens"])
    return answer(response_json(response), 0, 0, None, cache)


async def refund(limiter: Operations, cache: RecordingCache, call: RefundInput) -> JsonValue:
    await Operations.refund_reserved(
        limiter, call["scopes"], call["amount"], call["reservation_windows"]
    )
    return answer(None, 0, 0, None, cache)


async def reserve_io(limiter: Operations, cache: RecordingCache, call: ReserveIoInput) -> JsonValue:
    response, itpm_reserved, otpm_reserved = await limiter.reserve_io_tokens(
        call["descriptors"],
        call["estimated_input_tokens"],
        call["estimated_output_tokens"],
    )
    return answer(response_json(response), itpm_reserved, otpm_reserved, None, cache)


async def enforce_frame(limiter: Operations, cache: RecordingCache, call: FrameInput) -> JsonValue:
    fields = call["auth"]
    auth: UserAPIKeyAuth | None = None
    if fields is not None:
        auth = TypeAdapter(UserAPIKeyAuth).validate_python(fields)
    task: asyncio.Future[None] = asyncio.ensure_future(
        limiter.enforce_project_io_token_quota_for_frame(
            auth,
            call["requested_model"],
            call["estimated_input_tokens"],
            call["estimated_output_tokens"],
        )
    )
    _done, _pending = await asyncio.wait({task})
    error = task.exception()
    raised: JsonValue = None if error is None else raised_json(error)
    return answer(None, 0, 0, raised, cache)
