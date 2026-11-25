import datetime as dt
from collections.abc import Sequence
from typing import cast

from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.litellm_core_utils.logging_worker import GLOBAL_LOGGING_WORKER
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import PassThroughEndpointLoggingResultValues as LoggingResult
from litellm.proxy.common_request_processing import ProxyBaseLLMRequestProcessing
from litellm.proxy.pass_through_endpoints import streaming_handler
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    tinyfish_passthrough_logging_handler as tinyfish,
)
from litellm.proxy.pass_through_endpoints.streaming_handler import PassThroughStreamingHandler
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from litellm.types.utils import ModelResponse, StandardPassThroughResponseObject, Usage
from pydantic import JsonValue

from .model import COMPLETION_TOKENS, PROMPT_TOKENS, RESPONSE_COST, TOTAL_TOKENS
from .recorder import RECORDER

# the endpoints LiteLLM's reconstruction has a provider handler for; the rest fall back to a standard object
RECONSTRUCTED_ENDPOINTS = frozenset(
    {EndpointType.ANTHROPIC, EndpointType.VERTEX_AI, EndpointType.GEMINI, EndpointType.OPENAI}
)


def start_billing(
    run_id: str,
    litellm_logging_obj: LiteLLMLoggingObj,
    start_time: dt.datetime,
    client: AsyncHTTPHandler | None = None,
) -> None:
    _ = client
    RECORDER.started.append((run_id, start_time))
    if RECORDER.poller:
        tinyfish.mark_sse_poller_spawned(litellm_logging_obj)


def run_id_from_frames(frames: bytes) -> str | None:
    _ = frames
    RECORDER.scans += 1
    return RECORDER.run_id


def process_chunk(
    chunk: bytes, model_name: str, litellm_logging_obj: LiteLLMLoggingObj | None = None
) -> bytes:
    _ = (model_name, litellm_logging_obj)
    return b"injected:" + chunk


def build_result(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L377-L387
    *,
    litellm_logging_obj: LiteLLMLoggingObj,
    url_route: str,
    request_body: dict[str, JsonValue],
    endpoint_type: EndpointType,
    start_time: dt.datetime,
    raw_bytes: Sequence[bytes],
    end_time: dt.datetime,
    model: str | None,
    **kwargs: JsonValue,
) -> tuple[LoggingResult, dict[str, float]]:
    _ = (
        litellm_logging_obj,
        url_route,
        request_body,
        endpoint_type,
        start_time,
        raw_bytes,
        end_time,
        model,
        kwargs,
    )
    if endpoint_type not in RECONSTRUCTED_ENDPOINTS:
        return StandardPassThroughResponseObject(response="cannot parse chunks"), {}
    usage = Usage(
        prompt_tokens=PROMPT_TOKENS,
        completion_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS,
    )
    return ModelResponse(usage=usage), {"response_cost": RESPONSE_COST}


def install_fakes() -> None:
    GLOBAL_LOGGING_WORKER.ensure_initialized_and_enqueue = cast("object", RECORDER)  # type: ignore[assignment, method-assign]  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/logging_worker.py#L448-L453
    ProxyBaseLLMRequestProcessing._process_chunk_with_cost_injection = cast("object", process_chunk)  # type: ignore[assignment, method-assign, misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L4068-L4071
    tinyfish.TinyFishPassthroughLoggingHandler.start_sse_run_billing = start_billing  # type: ignore[method-assign]  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/tinyfish_passthrough_logging_handler.py#L203-L223
    PassThroughStreamingHandler._build_passthrough_logging_result = build_result  # type: ignore[assignment, method-assign]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L377-L387
    streaming_handler.run_id_from_sse_frames = run_id_from_frames  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/streaming_handler.py#L30-L33
