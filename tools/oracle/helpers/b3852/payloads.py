from datetime import UTC, datetime

from litellm.integrations.otel.logger import OpenTelemetryV2

from .wire import CALL_ID, END_SECONDS, MODEL, START_SECONDS, Fields, Params


def payload_metadata(auth_metadata: Params | None, tool_name: str | None) -> Fields:
    metadata: Fields = {}
    if tool_name is not None:
        metadata["mcp_tool_call_metadata"] = {"name": tool_name}
    if auth_metadata is not None:
        metadata["user_api_key_auth_metadata"] = dict(auth_metadata)
    return metadata


def payload_document(call_type: str, status: str, metadata: Fields) -> Fields:
    return {
        "id": CALL_ID,
        "litellm_call_id": CALL_ID,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": call_type,
        "response_cost": 0.5,
        "status": status,
        "status_fields": {"llm_api_status": status, "guardrail_status": "not_run"},
        "total_tokens": 5,
        "prompt_tokens": 2,
        "completion_tokens": 3,
        "startTime": START_SECONDS,
        "endTime": END_SECONDS,
        "completionStartTime": START_SECONDS + 0.5,
        "response_time": 1.0,
        "model_map_information": {"model_map_key": "", "model_map_value": None},
        "model": MODEL,
        "api_base": "https://api.openai.com",
        "metadata": metadata,
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "model_parameters": {},
        "hidden_params": {},
    }


def pre_call_details(credentials: Params | None, auth_metadata: Params | None) -> Fields:
    details: Fields = {
        "model": MODEL,
        "call_type": "completion",
        "litellm_call_id": CALL_ID,
    }
    if credentials is not None:
        details["standard_callback_dynamic_params"] = dict(credentials)
    if auth_metadata is not None:
        details["litellm_params"] = {
            "metadata": {"user_api_key_auth_metadata": dict(auth_metadata)}
        }
    return details


def close_kwargs(payload: Fields, metadata: Fields, credentials: Params | None) -> Fields:
    kwargs: Fields = {
        "standard_logging_object": payload,
        "litellm_call_id": CALL_ID,
        "model": MODEL,
        "call_type": payload["call_type"],
        "optional_params": {"stream": False},
        "litellm_params": {"metadata": metadata},
        "api_call_start_time": START_SECONDS,
    }
    if credentials is not None:
        kwargs["standard_callback_dynamic_params"] = dict(credentials)
    return kwargs


async def log_event(logger: OpenTelemetryV2, kwargs: Fields, *, failed: bool) -> None:
    started = datetime.fromtimestamp(START_SECONDS, tz=UTC)
    ended = datetime.fromtimestamp(END_SECONDS, tz=UTC)
    if failed:
        await logger.async_log_failure_event(  # type: ignore[no-untyped-call, misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L381
            kwargs,
            None,
            started,
            ended,
        )
        return
    await logger.async_log_success_event(  # type: ignore[no-untyped-call, misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L339
        kwargs,
        None,
        started,
        ended,
    )
