from collections.abc import Coroutine

from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._native import NativeAssistantRunSession
from betterlite.assistants._records import (
    Assistant,
    AssistantDeleted,
    AssistantPage,
    acreate_assistants,
    acreate_thread,
    adelete_assistant,
    aget_assistants,
    aget_thread,
    create_assistants,
    create_thread,
    delete_assistant,
    get_assistants,
    get_thread,
    named_arguments,
)
from betterlite.assistants._runs import (
    AssistantStream,
    AssistantStreamEvent,
    AsyncAssistantStream,
    EventHandler,
    Message,
    MessageAnswer,
    MessagePage,
    MessagePageAnswer,
    Run,
    RunAnswer,
    StreamAnswer,
    call_values,
    checked_handler,
    completed_message,
    message_payload,
    open_stream,
    run_payload,
    validated_aanswer,
    validated_answer,
)


def add_message(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L892
    custom_llm_provider: str, thread_id: str, role: str, content: str,
    attachments: CallValue = None, metadata: CallValue = None, client: CallValue = None,
    **kwargs: CallValue,
) -> MessageAnswer:
    arguments = named_arguments(
        kwargs,
        {"attachments": attachments, "metadata": metadata, "client": client},
    )
    if arguments.get("a_add_message") is True:
        return a_add_message(custom_llm_provider, thread_id, role, content, **arguments)
    payload = message_payload(custom_llm_provider, thread_id, role, content, arguments)
    message = validated_answer(Message, NativeAssistantRunSession.add_message, payload)
    return completed_message(message)


async def a_add_message(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L841
    custom_llm_provider: str, thread_id: str, role: str, content: str,
    attachments: CallValue = None, metadata: CallValue = None, client: CallValue = None,
    **kwargs: CallValue,
) -> Message:
    arguments = named_arguments(
        kwargs,
        {"attachments": attachments, "metadata": metadata, "client": client},
    )
    payload = message_payload(custom_llm_provider, thread_id, role, content, arguments)
    message = await validated_aanswer(
        Message, NativeAssistantRunSession.a_add_message, payload,
    )
    return completed_message(message)


def get_messages(
    custom_llm_provider: str, thread_id: str, client: CallValue = None,
    **kwargs: CallValue,
) -> MessagePageAnswer:
    arguments = named_arguments(kwargs, {"client": client})
    if arguments.get("aget_messages") is True:
        return aget_messages(custom_llm_provider, thread_id, **arguments)
    payload = to_call_payload(call_values(custom_llm_provider, thread_id, arguments))
    return validated_answer(MessagePage, NativeAssistantRunSession.get_messages, payload)


async def aget_messages(
    custom_llm_provider: str, thread_id: str, client: CallValue = None,
    **kwargs: CallValue,
) -> MessagePage:
    arguments = named_arguments(kwargs, {"client": client})
    payload = to_call_payload(call_values(custom_llm_provider, thread_id, arguments))
    return await validated_aanswer(
        MessagePage, NativeAssistantRunSession.aget_messages, payload,
    )


def run_thread(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1239
    custom_llm_provider: str, thread_id: str, assistant_id: str,
    additional_instructions: CallValue = None, instructions: CallValue = None,
    metadata: CallValue = None, model: CallValue = None, stream: CallValue = None,
    tools: CallValue = None, client: CallValue = None,
    event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> RunAnswer:
    asynchronous = kwargs.get("arun_thread") is True
    arguments = named_arguments(
        kwargs,
        {
            "additional_instructions": additional_instructions, "instructions": instructions,
            "metadata": metadata, "model": model, "tools": tools, "client": client,
        },
    )
    values: dict[str, CallValue] = {
        **arguments,
        "custom_llm_provider": custom_llm_provider,
        "thread_id": thread_id,
        "assistant_id": assistant_id,
    }
    handler = checked_handler(event_handler)
    payload = run_payload(values)
    if stream is True:
        return open_answer(payload, handler, asynchronous=asynchronous)
    if asynchronous:
        return validated_aanswer(Run, NativeAssistantRunSession.arun_thread, payload)
    return validated_answer(Run, NativeAssistantRunSession.run_thread, payload)


async def arun_thread(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1174
    custom_llm_provider: str, thread_id: str, assistant_id: str,
    additional_instructions: CallValue = None, instructions: CallValue = None,
    metadata: CallValue = None, model: CallValue = None, stream: CallValue = None,
    tools: CallValue = None, client: CallValue = None, **kwargs: CallValue,
) -> Run | AsyncAssistantStream:
    answer = run_thread(
        custom_llm_provider, thread_id, assistant_id,
        additional_instructions, instructions, metadata, model, stream, tools, client,
        arun_thread=True, **kwargs,
    )
    match answer:
        case AsyncAssistantStream() as opened:
            return opened
        case Coroutine() as pending:
            return await pending
        case object():
            raise TypeError("run_thread answered no run or stream")


def open_answer(
    payload: CallPayload, handler: EventHandler | None, *, asynchronous: bool,
) -> StreamAnswer:
    native = open_stream(payload, asynchronous=asynchronous)
    if asynchronous:
        return AsyncAssistantStream(native, handler)
    return AssistantStream(native, handler)


def run_thread_stream(
    *, event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> StreamAnswer:
    asynchronous = kwargs.get("arun_thread") is True
    payload = run_payload(kwargs)
    return open_answer(payload, checked_handler(event_handler), asynchronous=asynchronous)


def arun_thread_stream(
    *, event_handler: EventHandler | CallValue = None, **kwargs: CallValue,
) -> AsyncAssistantStream:
    payload = run_payload(kwargs)
    handler = checked_handler(event_handler)
    return AsyncAssistantStream(open_stream(payload, asynchronous=True), handler)


__all__ = [
    "Assistant", "AssistantDeleted", "AssistantPage", "AssistantStream",
    "AssistantStreamEvent", "AsyncAssistantStream", "EventHandler", "Message",
    "MessagePage", "Run", "a_add_message", "acreate_assistants",
    "acreate_thread", "add_message", "adelete_assistant", "aget_assistants",
    "aget_messages", "aget_thread", "arun_thread", "arun_thread_stream",
    "create_assistants", "create_thread", "delete_assistant", "get_assistants",
    "get_messages", "get_thread", "run_thread", "run_thread_stream",
]
