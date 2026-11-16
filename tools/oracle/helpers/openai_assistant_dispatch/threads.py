from litellm.llms.openai.openai import OpenAIAssistantsAPI
from pydantic import JsonValue

from .support import THREAD_BODY, Helpers, Input, Wire, async_client, credentials

THREADS: Helpers = OpenAIAssistantsAPI()


async def threads_answer(input_value: Input, wire: Wire) -> JsonValue:
    operation = input_value["operation"]
    client = async_client(input_value, wire)
    payload: dict[str, JsonValue]
    if operation == "create_thread":
        body = THREAD_BODY.validate_python(input_value["data"])
        thread = await THREADS.async_create_thread(
            **credentials(input_value), client=client, **body
        )
        payload = {"kind": "thread", "id": thread.id}
    elif operation == "retrieve_thread":
        found = await THREADS.async_get_thread(
            **credentials(input_value), client=client, thread_id=input_value["thread_id"]
        )
        payload = {"kind": "thread", "id": found.id}
    elif operation == "add_message":
        message = await THREADS.a_add_message(
            **credentials(input_value),
            client=client,
            thread_id=input_value["thread_id"],
            message_data=input_value["data"],
        )
        payload = {"kind": "message", "id": message.id, "status": message.status}
    else:
        page = await THREADS.async_get_messages(
            **credentials(input_value), client=client, thread_id=input_value["thread_id"]
        )
        ids: list[JsonValue] = [item.id for item in page.data]
        payload = {"kind": "page", "ids": ids}
    return payload
