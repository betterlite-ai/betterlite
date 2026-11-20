import asyncio

import litellm
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from bedrock_claude_invoke import Fetcher, patched_url_fetchers

from .views import CHAT, MESSAGES, OBJECT, ChatCall, MessagesCall, Object


def messages_request(call: MessagesCall) -> JsonValue:
    model_cost: Object = call["model_cost"]
    litellm.model_cost = dict(model_cost)
    litellm.drop_params = False
    headers = dict(call["request_headers"])
    body = MESSAGES.transform_anthropic_messages_request(
        call["model"],
        list(call["messages"]),
        dict(call["optional_params"]),
        GenericLiteLLMParams(),
        headers,
    )
    outcome: Object = {"body": body, "headers": OBJECT.validate_python(headers)}
    return outcome


def chat(call: ChatCall, *, inline: bool) -> JsonValue:
    model_cost: Object = call["model_cost"]
    litellm.register_model(model_cost)
    fetcher = Fetcher(dict(call["fetch"]))
    headers = dict(call["headers"])
    optional = dict(call["optional_params"])
    extra = dict(call["litellm_params"])
    messages = list(call["messages"])
    with patched_url_fetchers(fetcher):
        body = (
            asyncio.run(
                CHAT.async_transform_request(
                    call["model"], messages, optional, extra, headers
                )
            )
            if inline
            else CHAT.transform_request(
                call["model"], messages, optional, extra, headers
            )
        )
    outcome: Object = {"body": body, "fetches": fetcher.calls}
    return outcome
