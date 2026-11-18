import asyncio
from collections.abc import AsyncIterator

from litellm.llms.bedrock.realtime.handler import BedrockRealtime
from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from pydantic import JsonValue

from bedrock_services_realtime_session import make_logging

from .fakes import (
    BedrockAwaitError,
    BedrockReceiveError,
    BedrockSendError,
    Counter,
    FakeClient,
    FakeStream,
)
from .models import ErrorKind, JsonAdapter, Outcome, Run, RunResult
from .views import empty_state, scope_view

HANDLER = BedrockRealtime()


async def cancel_when(counter: Counter, task: asyncio.Task[None]) -> None:
    await counter.reached.wait()
    task.cancel()


async def stop_at(counter: Counter, task: asyncio.Task[None]) -> None:
    canceller = asyncio.create_task(cancel_when(counter, task))
    await asyncio.wait({task, canceller}, return_when=asyncio.FIRST_COMPLETED)
    canceller.cancel()


async def consume(events: AsyncIterator[object], logged: list[JsonValue]) -> None:
    async for event in events:
        logged.append(JsonAdapter.validate_python(event))  # noqa: PERF401 - a later failure keeps the events already logged  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L581-L589


async def client_run(run_value: Run, model: str, config: BedrockRealtimeConfig) -> RunResult:
    state = empty_state()
    client = FakeClient(run_value)
    stream = FakeStream(run_value)
    trace_id = run_value["trace_id"]
    logging_obj = None if trace_id is None else make_logging(trace_id)
    pump = HANDLER._forward_client_to_bedrock(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L459-L511
        client,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L89-L98
        stream,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L121-L125
        config,
        model,
        state,
        logging_obj,
        run_value["initial"],
    )
    task = asyncio.create_task(pump)
    outcome: Outcome = "ended"
    error: ErrorKind | None = None
    if run_value["cancel"]:
        await stop_at(client.counter, task)
    try:
        await task
    except asyncio.CancelledError:
        outcome = "cancelled"
    except BedrockAwaitError:
        outcome, error = "error", "ack"
    except BedrockSendError:
        outcome, error = "error", "stream"
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L459-L511
        outcome, error = "error", "input"
    return {
        "outcome": outcome,
        "error": error,
        "sent_to_bedrock": stream.input.sent,
        "sent_to_client": client.sent,
        "closed_input": stream.input.closed,
        "closed_client": client.closed,
        "scope": scope_view(client.scope),
        "state": None,
        "logged": [],
    }


async def provider_run(run_value: Run, model: str, config: BedrockRealtimeConfig) -> RunResult:
    state = empty_state()
    client = FakeClient(run_value)
    stream = FakeStream(run_value)
    trace_id = run_value["trace_id"]
    logging_obj = make_logging("trace-bridge-1" if trace_id is None else trace_id)
    events: AsyncIterator[object] = HANDLER._forward_bedrock_to_client(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L513-L589
        stream,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L121-L125
        client,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L89-L98
        config,
        model,
        logging_obj,
        state,
    )
    logged: list[JsonValue] = []
    task = asyncio.create_task(consume(events, logged))
    outcome: Outcome = "ended"
    error: ErrorKind | None = None
    if run_value["cancel"]:
        await stop_at(stream.output.counter, task)
    try:
        await task
    except asyncio.CancelledError:
        outcome = "cancelled"
    except (BedrockAwaitError, BedrockReceiveError):
        outcome, error = "error", "stream"
    except UnicodeDecodeError:
        outcome, error = "error", "utf8"
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/realtime/handler.py#L513-L589
        outcome, error = "error", "transform"
    return {
        "outcome": outcome,
        "error": error,
        "sent_to_bedrock": [],
        "sent_to_client": client.sent,
        "closed_input": False,
        "closed_client": client.closed,
        "scope": None,
        "state": JsonAdapter.validate_python(state),
        "logged": logged,
    }


async def bridge_run(run_value: Run, model: str) -> RunResult:
    config = BedrockRealtimeConfig()
    if run_value["kind"] == "client":
        return await client_run(run_value, model, config)
    return await provider_run(run_value, model, config)
