import asyncio

from pydantic import JsonValue

from .assistants import API
from .types import JSON, ORG, Input
from .wire import Wire, async_client, sync_client


def buffered_result(input_value: Input, wire: Wire) -> JsonValue:
    body = input_value["request"]
    if input_value["arun_thread"]:
        run = asyncio.run(
            API.run_thread(
                api_key=input_value["api_key"], api_base=input_value["api_base"],
                timeout=30.0, max_retries=0, organization=ORG,
                thread_id=body["thread_id"], assistant_id=body["assistant_id"],
                additional_instructions=body["additional_instructions"],
                instructions=body["instructions"], metadata=body["metadata"],
                model=body["model"], stream=False, tools=body["tools"],
                client=async_client(input_value, wire), arun_thread=True, event_handler=None,
            )
        )
    else:
        run = API.run_thread(
            api_key=input_value["api_key"], api_base=input_value["api_base"],
            timeout=30.0, max_retries=0, organization=ORG,
            thread_id=body["thread_id"], assistant_id=body["assistant_id"],
            additional_instructions=body["additional_instructions"],
            instructions=body["instructions"], metadata=body["metadata"],
            model=body["model"], stream=False, tools=body["tools"],
            client=sync_client(input_value, wire), arun_thread=False, event_handler=None,
        )
    payload: dict[str, JsonValue] = {"kind": "polled", "id": run.id, "status": run.status}
    return JSON.validate_python(payload)


async def stream_names(input_value: Input, wire: Wire) -> list[JsonValue]:
    body = input_value["request"]
    manager = API.async_run_thread_stream(
        client=async_client(input_value, wire), thread_id=body["thread_id"],
        assistant_id=body["assistant_id"],
        additional_instructions=body["additional_instructions"],
        instructions=body["instructions"], metadata=body["metadata"], model=body["model"],
        tools=body["tools"], event_handler=None,
    )
    async with manager as stream:
        names: list[JsonValue] = [event.event async for event in stream]
    return names


def runs_result(input_value: Input, wire: Wire) -> JsonValue:
    if input_value["operation"] == "buffered":
        return buffered_result(input_value, wire)
    names = asyncio.run(stream_names(input_value, wire))
    payload: dict[str, JsonValue] = {"kind": "stream", "events": names}
    return JSON.validate_python(payload)
