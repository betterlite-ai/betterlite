from typing import Literal, TypedDict

from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue

from .support import EMPTY, EMPTY_MESSAGES, Body, ChatConfig, config

# pydantic leaves a plain dict message alone, and LiteLLM reads the prompt from exactly these fields
PROMPT: list[AllMessageValues] = [
    {"role": "user", "content": "hello there"},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
    {"role": "user", "content": "second"},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
]


class RequestCase(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str]
    non_default_params: Body
    optional_params: Body


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


class EmptyInput(TypedDict):
    op: Literal["request_empty"]


def request_view(chat: ChatConfig, case: RequestCase) -> JsonValue:
    optional_params: Body = dict(case["optional_params"])
    chat.map_openai_params(
        non_default_params=case["non_default_params"],
        optional_params=optional_params,
        model=case["model"],
        drop_params=False,
    )
    headers: Body = chat.validate_environment(
        headers=case["headers"],
        model=case["model"],
        messages=EMPTY_MESSAGES,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=optional_params,
        litellm_params=EMPTY,
        api_key=case["api_key"],
        api_base=case["api_base"],
    )
    url = chat.get_complete_url(
        api_base=case["api_base"],
        api_key=case["api_key"],
        model=case["model"],
        optional_params=optional_params,
        litellm_params=EMPTY,
        stream=None,
    )
    body: Body = chat.transform_request(
        model=case["model"],
        messages=PROMPT,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=optional_params,
        litellm_params=EMPTY,
        headers=headers,
    )
    return {"url": url, "headers": headers, "body": body}


def request(input_value: RequestInput) -> JsonValue:
    chat = config()
    return {"cases": [request_view(chat, case) for case in input_value["cases"]]}


def empty_request() -> JsonValue:
    chat = config()
    try:
        chat.transform_request(
            model="my-agent",
            messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=EMPTY,
            litellm_params=EMPTY,
            headers=EMPTY,
        )
    except ValueError as error:
        return {"raised": str(error)}
    return {"raised": None}
