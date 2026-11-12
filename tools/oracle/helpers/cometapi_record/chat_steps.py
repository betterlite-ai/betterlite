from typing import cast

from litellm.llms.cometapi.chat.transformation import (
    CometAPIChatCompletionStreamingHandler as CometStreamHandler,
)
from litellm.llms.cometapi.chat.transformation import CometAPIConfig
from litellm.llms.cometapi.common_utils import CometAPIException
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue

from .inputs import EMPTY, ChunkStep, OptionsStep, RequestStep, UrlStep, apply_env


def options(config: CometAPIConfig, step: OptionsStep) -> JsonValue:
    params: dict[str, JsonValue] = dict(step["params"])
    optional: dict[str, JsonValue] = dict(step["optional_params"])
    supported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L143-L185
        "list[JsonValue]",
        config.get_supported_openai_params(step["model"]),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L143-L185
    )
    mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L219-L231
        "dict[str, JsonValue]",
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L219-L231
            non_default_params=params,
            optional_params=optional,
            model=step["model"],
            drop_params=False,
        ),
    )
    return {"supported": supported, "mapped": mapped}


def request(config: CometAPIConfig, step: RequestStep) -> JsonValue:
    apply_env(step["env"])
    optional: dict[str, JsonValue] = dict(step["optional_params"])
    try:
        body = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L67-L84
            model=step["model"],
            messages=cast("list[AllMessageValues]", step["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=optional,
            litellm_params=EMPTY,
            headers=EMPTY,
        )
    except (TypeError, ValueError):
        return {"ok": False}
    return {"ok": True, "value": cast("dict[str, JsonValue]", body)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L67-L84


def url(config: CometAPIConfig, step: UrlStep) -> JsonValue:
    complete = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L86-L124
        api_base=step["api_base"],
        api_key=None,
        model="m",
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    return {"url": complete}


def chunk_value(parsed: ModelResponseStream) -> JsonValue:
    dumped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
        "dict[str, JsonValue]",
        parsed.model_dump(),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103
    )
    usage: JsonValue = None
    if dumped["usage"] is not None:
        counts = cast("dict[str, JsonValue]", dumped["usage"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1857
        usage = {
            "prompt_tokens": counts.get("prompt_tokens"),
            "completion_tokens": counts.get("completion_tokens"),
            "total_tokens": counts.get("total_tokens"),
        }
    choices = cast("list[dict[str, JsonValue]]", dumped["choices"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006
    projected: list[JsonValue] = []
    for choice in choices:
        delta = cast("dict[str, JsonValue]", choice["delta"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1507
        projected.append(
            {
                "index": choice.get("index"),
                "content": delta.get("content"),
                "reasoning": delta.get("reasoning"),
                "reasoning_content": delta.get("reasoning_content"),
            }
        )
    return {
        "id": dumped["id"],
        "object": dumped["object"],
        "created": dumped["created"],
        "model": dumped["model"],
        "usage": usage,
        "choices": projected,
    }


def chunk(handler: CometStreamHandler, step: ChunkStep) -> JsonValue:
    payload: dict[str, JsonValue] = dict(step["chunk"])
    try:
        parsed = handler.chunk_parser(payload)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/chat/transformation.py#L157-L195
    except CometAPIException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/common_utils.py#L4
        return {"ok": False, "class": "CometAPIException", "status": error.status_code}
    return {"ok": True, "value": chunk_value(parsed)}
