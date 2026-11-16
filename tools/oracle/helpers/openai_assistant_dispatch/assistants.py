import asyncio

from litellm.llms.openai.openai import OpenAIAssistantsAPI
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta.assistant import Assistant
from pydantic import JsonValue

from .calls import call_params
from .types import JSON, ORG, THREAD_CREATE, Input
from .wire import Wire, async_client, sync_client

API = OpenAIAssistantsAPI()


def assistants_result(input_value: Input, wire: Wire) -> JsonValue:
    operation = input_value["operation"]
    if operation == "async_list":
        params = input_value["params"]
        page: AsyncCursorPage[Assistant] | SyncCursorPage[Assistant]
        if input_value["aget_assistants"] is None:
            page = asyncio.run(
                API.async_get_assistants(
                    **call_params(input_value), client=async_client(input_value, wire),
                    order=params["order"], limit=params["limit"],
                    before=params["before"], after=params["after"],
                )
            )
        elif input_value["aget_assistants"]:
            # the facade keeps the async client but drops the caller's pagination for this branch
            page = asyncio.run(
                API.get_assistants(
                    api_key=input_value["api_key"], api_base=input_value["api_base"],
                    timeout=30.0, max_retries=0, organization=ORG,
                    client=async_client(input_value, wire), aget_assistants=True,
                )
            )
        else:
            page = API.get_assistants(
                api_key=input_value["api_key"], api_base=input_value["api_base"],
                timeout=30.0, max_retries=0, organization=ORG,
                client=sync_client(input_value, wire), aget_assistants=False,
            )
        ids: list[JsonValue] = [item.id for item in page.data]
        payload: dict[str, JsonValue] = {"ids": ids, "has_more": page.has_more}
        return JSON.validate_python(payload)
    if operation == "create":
        created = asyncio.run(
            API.async_create_assistants(
                **call_params(input_value), client=async_client(input_value, wire),
                create_assistant_data=input_value["data"],
            )
        )
        assistant: dict[str, JsonValue] = {"id": created.id, "object": created.object}
        return JSON.validate_python(assistant)
    deleted = asyncio.run(
        API.async_delete_assistant(
            **call_params(input_value), client=async_client(input_value, wire),
            assistant_id=input_value["assistant_id"],
        )
    )
    deletion: dict[str, JsonValue] = {"id": deleted.id, "deleted": deleted.deleted, "object": deleted.object}
    return JSON.validate_python(deletion)


def threads_result(input_value: Input, wire: Wire) -> JsonValue:
    operation = input_value["operation"]
    if operation == "create":
        body = THREAD_CREATE.validate_python(input_value["data"])
        thread = asyncio.run(
            API.async_create_thread(
                **call_params(input_value), client=async_client(input_value, wire),
                messages=body["messages"], metadata=body["metadata"],
            )
        )
        created: dict[str, JsonValue] = {"kind": "thread", "id": thread.id}
        return JSON.validate_python(created)
    thread_id = input_value["thread_id"]
    if operation == "add_message":
        message = asyncio.run(
            API.a_add_message(
                **call_params(input_value), client=async_client(input_value, wire),
                thread_id=thread_id, message_data=input_value["data"],
            )
        )
        added: dict[str, JsonValue] = {"kind": "message", "id": message.id, "status": message.status}
        return JSON.validate_python(added)
    if operation == "list_messages":
        listed = asyncio.run(
            API.async_get_messages(
                **call_params(input_value), client=async_client(input_value, wire), thread_id=thread_id,
            )
        )
        message_ids: list[JsonValue] = [item.id for item in listed.data]
        page_payload: dict[str, JsonValue] = {"kind": "page", "ids": message_ids}
        return JSON.validate_python(page_payload)
    retrieved = asyncio.run(
        API.async_get_thread(
            **call_params(input_value), client=async_client(input_value, wire), thread_id=thread_id,
        )
    )
    found: dict[str, JsonValue] = {"kind": "thread", "id": retrieved.id}
    return JSON.validate_python(found)
