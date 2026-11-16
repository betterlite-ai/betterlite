from typing import Literal, TypedDict

import httpx
from litellm.llms.a2a.common_utils import A2AError
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter

from .support import EMPTY, EMPTY_MESSAGES, ChatConfig, config, error_view, logging_object


class ReplyMessage(TypedDict):
    role: str
    content: str | None


class ReplyChoice(TypedDict):
    index: int
    finish_reason: str
    message: ReplyMessage


class ReplyView(TypedDict):
    id: str
    model: str | None
    choices: list[ReplyChoice]


class ResponseCase(TypedDict):
    model: str
    status: int
    body: str


class ResponseInput(TypedDict):
    op: Literal["response"]
    cases: list[ResponseCase]


def raw_response(case: ResponseCase) -> httpx.Response:
    return httpx.Response(
        case["status"],
        content=case["body"].encode(),
        headers={"content-type": "application/json"},
        request=httpx.Request("POST", "http://a2a.test/agent"),
    )


def reply_view(reply: ModelResponse) -> JsonValue:
    fields: ReplyView = TypeAdapter(ReplyView).validate_json(reply.model_dump_json())
    choice = fields["choices"][0]
    return {
        "id": fields["id"],
        "model": fields["model"],
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "role": choice["message"]["role"],
        "content": choice["message"]["content"],
    }


def response_view(chat: ChatConfig, case: ResponseCase) -> JsonValue:
    try:
        reply = chat.transform_response(
            model=case["model"],
            raw_response=raw_response(case),
            model_response=ModelResponse(),
            logging_obj=logging_object(),
            request_data=EMPTY,
            messages=EMPTY_MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=EMPTY,
            litellm_params=EMPTY,
            encoding=None,
        )
    except A2AError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/a2a/common_utils.py#L18
        return error_view(error)
    return {"reply": reply_view(reply)}


def response(input_value: ResponseInput) -> JsonValue:
    chat = config()
    return {"cases": [response_view(chat, case) for case in input_value["cases"]]}
