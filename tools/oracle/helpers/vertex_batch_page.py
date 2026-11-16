from typing import Final, TypedDict, cast

import httpx
from pydantic import JsonValue as Json

from litellm.llms.vertex_ai.batches.handler import (  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/handler.py#L79-L80
    _vertex_batch_list_payload,
)

STATUS_OK: Final[int] = 200


class Input(TypedDict):
    body: str


def run(input_value: Input) -> Json:
    response = httpx.Response(STATUS_OK, content=input_value["body"].encode())
    payload = _vertex_batch_list_payload(  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/handler.py#L79-L80
        response
    )
    return cast(Json, payload)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/batches/handler.py#L79-L80
