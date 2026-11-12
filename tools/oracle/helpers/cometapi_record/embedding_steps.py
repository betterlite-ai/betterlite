from typing import cast

import httpx
from litellm.llms.cometapi.common_utils import CometAPIException
from litellm.llms.cometapi.embed.transformation import CometAPIEmbeddingConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse
from pydantic import JsonValue

from .inputs import (
    EMPTY,
    NO_MESSAGES,
    EmbeddingHeadersStep,
    EmbeddingOptionsStep,
    EmbeddingReplyStep,
    EmbeddingRequestStep,
    EmbeddingUrlStep,
    HeaderLookup,
    apply_env,
)


def embedding_headers(
    config: CometAPIEmbeddingConfig, step: EmbeddingHeadersStep
) -> JsonValue:
    apply_env(step["env"])
    headers: dict[str, str] = dict(step["headers"])
    validated = config.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L46-L71
        headers=headers,
        model="m",
        messages=cast("list[AllMessageValues]", NO_MESSAGES),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
        optional_params=EMPTY,
        litellm_params=EMPTY,
        api_key=step["api_key"],
        api_base=None,
    )
    return {"headers": cast("dict[str, JsonValue]", validated)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L46-L71


def embedding_url(config: CometAPIEmbeddingConfig, step: EmbeddingUrlStep) -> JsonValue:
    complete = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L30-L44
        api_base=step["api_base"],
        api_key=None,
        model="m",
        optional_params=EMPTY,
        litellm_params=EMPTY,
    )
    return {"url": complete}


def embedding_options(
    config: CometAPIEmbeddingConfig, step: EmbeddingOptionsStep
) -> JsonValue:
    params: dict[str, JsonValue] = dict(step["params"])
    optional: dict[str, JsonValue] = dict(step["optional_params"])
    mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L84-L98
        "dict[str, JsonValue]",
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L84-L98
            non_default_params=params,
            optional_params=optional,
            model="m",
            drop_params=True,
        ),
    )
    return {"mapped": mapped}


def embedding_request(
    config: CometAPIEmbeddingConfig, step: EmbeddingRequestStep
) -> JsonValue:
    optional: dict[str, JsonValue] = dict(step["optional_params"])
    body = config.transform_embedding_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L100-L110
        model=step["model"],
        input=step["input"],
        optional_params=optional,
        headers=EMPTY,
    )
    return {"ok": True, "value": cast("dict[str, JsonValue]", body)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L100-L110


def embedding_value(result: EmbeddingResponse) -> JsonValue:
    dumped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298
        "dict[str, JsonValue]",
        result.model_dump(),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298
    )
    usage = cast("dict[str, JsonValue]", dumped["usage"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L139-L144
    return {
        "model": dumped["model"],
        "data": dumped["data"],
        "object": dumped["object"],
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "total_tokens": usage["total_tokens"],
        },
    }


def embedding_reply(
    config: CometAPIEmbeddingConfig, step: EmbeddingReplyStep
) -> JsonValue:
    response = httpx.Response(
        step["status"],
        content=step["body"].encode(),
        headers=step["headers"],
        request=httpx.Request("POST", "https://host"),
    )
    try:
        result = config.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cometapi/embed/transformation.py#L112-L145
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
        carried = cast(HeaderLookup, error.headers)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L55
        return {
            "ok": False,
            "class": "CometAPIException",
            "status": error.status_code,
            "headers": {name: carried.get(name) for name in step["headers"]},
        }
    return {"ok": True, "value": embedding_value(result)}
