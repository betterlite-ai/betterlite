import asyncio

import litellm
from litellm.constants import (
    BEDROCK_REALTIME_COMMITTED_FAILURE_SCOPE_KEY as FAILURE_KEY,
)
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.realtime import handler as bedrock_handler
from pydantic import JsonValue, ValidationError

from bedrock_services_realtime_session import make_logging

from .fakes import (
    FakeClient,
    FakeStream,
    ProviderError,
    provider_error,
    record_of,
    scope_of,
)
from .models import (
    MODEL,
    AckStep,
    ClientMessagesStep,
    DispatchStep,
    JsonAdapter,
    JsonObject,
    ParseClientStep,
    PendingStep,
    ProviderStep,
    RealtimeConfig,
    ShouldLogStep,
    StateAdapter,
)


def dispatch_step(
    config: RealtimeConfig, step: DispatchStep, logging_obj: Logging
) -> JsonObject:
    state = StateAdapter.validate_python(step["state"])
    try:
        response = config.transform_realtime_response(step["message"], MODEL, logging_obj, state)
    except ValidationError:
        return {"response": None, "raised": "invalid_payload"}
    except AttributeError:
        return {"response": None, "raised": "not_an_object"}
    except TypeError:
        return {"response": None, "raised": "event_not_an_object"}
    return {"response": response, "raised": None}


async def collect_messages(client: FakeClient, initial: str | None) -> JsonObject:
    generator = bedrock_handler._client_messages(client, initial)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L167-L175
    messages = [message async for message in generator]
    after_end: str | None = await anext(generator, None)
    return {
        "messages": JsonAdapter.validate_python(messages),
        "receive_calls": client.receive_calls,
        "after_end": after_end,
    }


def client_messages_step(step: ClientMessagesStep) -> JsonObject:
    client = FakeClient({}, list(step["received"]), None)
    return asyncio.run(collect_messages(client, step["initial"]))


def parse_client_step(step: ParseClientStep) -> JsonObject:
    parsed = bedrock_handler._parse_client_message(step["message"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L201-L205
    return {"parsed": JsonAdapter.validate_python(parsed)}


def pending_step(step: PendingStep) -> JsonObject:
    scope = scope_of(step["scope"])
    try:
        pending = bedrock_handler._pending_session_update(scope)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L178-L190
    except BedrockError as error:
        return {"pending": None, "status": error.status_code}
    return {"pending": pending, "status": None}


def provider_step(step: ProviderStep) -> JsonObject:
    failure = provider_error(step["class_name"], step["message"])
    scope = scope_of(step["scope"])
    try:
        bedrock_handler._raise_provider_failure(scope, failure)  # type: ignore[arg-type]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L193-L198
    except BedrockError as error:
        return {
            "status": error.status_code,
            "recorded": scope.get(FAILURE_KEY) is not None,
            "unchanged": False,
        }
    except ProviderError:
        return {
            "status": None,
            "recorded": scope.get(FAILURE_KEY) is not None,
            "unchanged": True,
        }


def ack_step(config: RealtimeConfig, step: AckStep) -> JsonObject:
    scope = scope_of(step["scope"])
    client = FakeClient(scope, [], step["send_error"])
    stream = FakeStream(step["stream_error"])
    trace_id = step["trace_id"]
    logging_obj = None if trace_id is None else make_logging(trace_id)
    try:
        acked = asyncio.run(
            bedrock_handler._ack_session_update(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L208-L232
                client,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L89-L98
                stream,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L121-L125
                config,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/transformation.py#L80
                MODEL,
                logging_obj,
                step["client_message"],
            )
        )
    except RuntimeError:
        return {"acked": None, "raised": "stream", "scope": record_of(scope), "sent": None}
    except ValidationError:
        return {"acked": None, "raised": "modalities", "scope": record_of(scope), "sent": None}
    sent: JsonValue = None
    if client.sent:
        sent = JsonAdapter.validate_json(client.sent[0])
    return {
        "acked": acked,
        "raised": None,
        "scope": record_of(scope),
        "sent": sent,
    }


def should_log_step(step: ShouldLogStep) -> JsonObject:
    litellm.logged_real_time_event_types = step["configured"]
    logged = bedrock_handler._should_log_event(step["event"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L78-L86
    return {"logged": logged}
