import asyncio

from litellm.llms.openai.openai import OpenAIAssistantsAPI
from openai.pagination import AsyncCursorPage, SyncCursorPage
from openai.types.beta import Assistant
from pydantic import JsonValue

from .support import Input, Wire, async_client, credentials, sync_client

HELPERS: OpenAIAssistantsAPI = OpenAIAssistantsAPI()


def assistants_answer(input_value: Input, wire: Wire) -> JsonValue:
    operation = input_value["operation"]
    payload: dict[str, JsonValue]
    if operation == "create_assistant":
        created = asyncio.run(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L2417-L2426
            HELPERS.async_create_assistants(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L2417-L2426
                **credentials(input_value),
                client=async_client(input_value, wire),
                create_assistant_data=input_value["data"],
            )
        )
        payload = {"id": created.id, "object": created.object}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L2417-L2426
    elif operation == "delete_assistant":
        deleted = asyncio.run(
            HELPERS.async_delete_assistant(
                **credentials(input_value),
                client=async_client(input_value, wire),
                assistant_id=input_value["assistant_id"],
            )
        )
        payload = {"id": deleted.id, "deleted": deleted.deleted, "object": deleted.object}
    else:
        page: AsyncCursorPage[Assistant] | SyncCursorPage[Assistant]
        aget_assistants = input_value["aget_assistants"]
        if aget_assistants is None:
            listed = HELPERS.async_get_assistants(
                **credentials(input_value),
                client=async_client(input_value, wire),
                **input_value["params"],
            )
            page = asyncio.run(listed)
        elif aget_assistants:
            # the async facade drops the caller's pagination and lists with the SDK defaults
            facade = HELPERS.get_assistants(
                **credentials(input_value),
                client=async_client(input_value, wire),
                aget_assistants=True,
            )
            page = asyncio.run(facade)
        else:
            page = HELPERS.get_assistants(
                **credentials(input_value),
                client=sync_client(input_value, wire),
                aget_assistants=False,
            )
        ids: list[JsonValue] = [item.id for item in page.data]
        payload = {"ids": ids, "has_more": page.has_more}
    return payload
