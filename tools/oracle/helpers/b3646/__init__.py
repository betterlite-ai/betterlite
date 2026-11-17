from datetime import UTC, datetime
from typing import TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    transcribe_passthrough_logging_handler as handler,
)
from pydantic import BaseModel


class Input(TypedDict):
    target: str | None


class Output(TypedDict):
    operation: str
    priced: bool


class PayloadKwargs(BaseModel):
    model: str


class TranscribePayload(BaseModel):
    kwargs: PayloadKwargs


def run(input_value: Input) -> Output:
    target = input_value["target"]
    headers = httpx.Headers() if target is None else httpx.Headers({"x-amz-target": target})
    request = httpx.Request(
        "POST",
        "https://transcribe.us-east-1.amazonaws.com/",
        headers=headers,
    )
    response = httpx.Response(200, request=request)
    started = datetime(2026, 1, 1, tzinfo=UTC)
    logging_obj = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=started,
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    payload = TranscribePayload.model_validate(
        handler.TranscribePassthroughLoggingHandler.transcribe_passthrough_handler(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L5066
            httpx_response=response,
            logging_obj=logging_obj,
            url_route="/v1/transcribe",
            result="{}",
            start_time=started,
            end_time=started,
            cache_hit=False,
            request_body={},
        )
    )
    model = payload.kwargs.model
    return {
        "operation": model.removeprefix(f"{handler.TRANSCRIBE_CUSTOM_LLM_PROVIDER}/"),
        "priced": handler.TranscribePassthroughLoggingHandler.is_priced_job_start(response),
    }
