import os
from typing import TypedDict, cast

import httpx
import litellm
from litellm.llms.cometapi.chat.transformation import (
    CometAPIChatCompletionStreamingHandler as CometStreamHandler,
)
from litellm.llms.cometapi.chat.transformation import CometAPIConfig
from litellm.llms.cometapi.common_utils import CometAPIException
from litellm.llms.cometapi.embed.transformation import CometAPIEmbeddingConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue

litellm.suppress_debug_info = True
EMPTY: dict[str, JsonValue] = {}
NO_MESSAGES: list[dict[str, JsonValue]] = []


class RequestCall(TypedDict):
    api_base: str | None
    model: str
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    env: dict[str, str]


class HeaderCall(TypedDict):
    api_key: str | None
    headers: dict[str, str]
    env: dict[str, str]


class ReplyCall(TypedDict):
    body: str
    status: int
    headers: dict[str, str]


class Input(TypedDict):
    function: str
    requests: list[RequestCall]
    chunks: list[dict[str, JsonValue]]
    calls: list[HeaderCall]
    replies: list[ReplyCall]


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value


def request_value(config: CometAPIConfig, call: RequestCall) -> JsonValue:
    apply_env(call["env"])
    url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L86-L124
        api_base=call["api_base"],
        api_key=None,
        model=call["model"],
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    optional: dict[str, JsonValue] = dict(call["optional_params"])
    try:
        body = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L67-L84
            model=call["model"],
            messages=cast("list[AllMessageValues]", call["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=optional,
            litellm_params=EMPTY,
            headers=EMPTY,
        )
    except (TypeError, ValueError):
        return {"url": url, "ok": False}
    return {"url": url, "ok": True, "value": cast("dict[str, JsonValue]", body)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L67-L84


def chunk_value(handler: CometStreamHandler, chunk: dict[str, JsonValue]) -> JsonValue:
    try:
        parsed = handler.chunk_parser(chunk)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L157-L195
    except CometAPIException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/common_utils.py#L4
        return {"ok": False, "class": "CometAPIException", "status": error.status_code}
    return {
        "ok": True,
        "value": {
            "id": parsed.id,
            "object": parsed.object,
            "created": parsed.created,
            "model": parsed.model,
            "usage": chunk.get("usage"),
            "choices": chunk["choices"],
        },
    }


def header_value(config: CometAPIEmbeddingConfig, call: HeaderCall) -> JsonValue:
    apply_env(call["env"])
    headers: dict[str, str] = dict(call["headers"])
    validated = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L46-L71
        headers=headers,
        model="m",
        messages=cast("list[AllMessageValues]", NO_MESSAGES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=EMPTY,
        api_key=call["api_key"],
        api_base=None,
    )
    return {"headers": cast("dict[str, JsonValue]", validated)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L46-L71


def reply_value(config: CometAPIEmbeddingConfig, call: ReplyCall) -> JsonValue:
    response = httpx.Response(
        call["status"],
        content=call["body"].encode(),
        headers=call["headers"],
        request=httpx.Request("POST", "https://host"),
    )
    try:
        result = config.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L112-L145
            model="m",
            raw_response=response,
            model_response=EmbeddingResponse(),
            logging_obj=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L116
            api_key=None,
            request_data=EMPTY,
            optional_params=EMPTY,
            litellm_params=EMPTY,
        )
    except CometAPIException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/common_utils.py#L4
        return {"ok": False, "class": "CometAPIException", "status": error.status_code}
    dumped = cast("dict[str, JsonValue]", result.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298
    usage = cast("dict[str, JsonValue]", dumped["usage"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L139-L144
    return {
        "ok": True,
        "value": {
            "model": dumped["model"],
            "data": dumped["data"],
            "object": dumped["object"],
            "usage": {
                "prompt_tokens": usage["prompt_tokens"],
                "total_tokens": usage["total_tokens"],
            },
        },
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "request":
        config = CometAPIConfig()
        return [request_value(config, call) for call in input_value["requests"]]
    if input_value["function"] == "chunks":
        handler = CometStreamHandler(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_model_iterator.py#L67-L71
            streaming_response=iter([""]), sync_stream=False, json_mode=False
        )
        return [chunk_value(handler, chunk) for chunk in input_value["chunks"]]
    embed = CometAPIEmbeddingConfig()
    if input_value["function"] == "embedding_headers":
        return [header_value(embed, call) for call in input_value["calls"]]
    return [reply_value(embed, call) for call in input_value["replies"]]
