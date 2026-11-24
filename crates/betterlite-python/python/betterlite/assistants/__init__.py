import sys

from betterlite.assistants import _utils as utils
from betterlite.assistants.main import (
    Assistant,
    AssistantDeleted,
    AssistantPage,
    AssistantStream,
    AssistantStreamEvent,
    AsyncAssistantStream,
    EventHandler,
    Message,
    MessagePage,
    Run,
    a_add_message,
    acreate_assistants,
    acreate_thread,
    add_message,
    adelete_assistant,
    aget_assistants,
    aget_messages,
    aget_thread,
    arun_thread,
    arun_thread_stream,
    create_assistants,
    create_thread,
    delete_assistant,
    get_assistants,
    get_messages,
    get_thread,
    run_thread,
    run_thread_stream,
)

__all__ = [
    "Assistant", "AssistantDeleted", "AssistantPage", "AssistantStream",
    "AssistantStreamEvent", "AsyncAssistantStream", "EventHandler", "Message",
    "MessagePage", "Run", "a_add_message", "acreate_assistants",
    "acreate_thread", "add_message", "adelete_assistant", "aget_assistants",
    "aget_messages", "aget_thread", "arun_thread", "arun_thread_stream",
    "create_assistants", "create_thread", "delete_assistant", "get_assistants",
    "get_messages", "get_thread", "run_thread", "run_thread_stream",
]

sys.modules[f"{__name__}.utils"] = utils
