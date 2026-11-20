from datetime import UTC, datetime

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.passthrough.transformation import BedrockPassthroughConfig
from litellm.llms.bedrock_mantle.passthrough.transformation import (
    BedrockMantlePassthroughConfig,
)
from litellm.types.utils import CostResponseTypes
from pydantic import JsonValue
from typing_extensions import override

from .support import JSON, REPLY_PAYLOAD


class RecordingPassthrough(BedrockPassthroughConfig):
    def __init__(self) -> None:
        super().__init__()
        self.forwarded: list[JsonValue] = []

    @override
    def logging_non_streaming_response(
        self,
        model: str,
        custom_llm_provider: str,
        httpx_response: httpx.Response,
        request_data: dict[str, JsonValue],
        logging_obj: Logging,
        endpoint: str,
    ) -> CostResponseTypes | None:
        self.forwarded.append(
            {
                "model": model,
                "provider": custom_llm_provider,
                "endpoint": endpoint,
                "request_data": JSON.validate_python(request_data),
            }
        )
        return None


class RecordingMantlePassthrough(BedrockMantlePassthroughConfig, RecordingPassthrough):
    pass


def reply_cases(payload: JsonValue) -> JsonValue:
    parsed = REPLY_PAYLOAD.validate_python(payload)
    logging_obj = Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="bedrock-mantle-call",
        function_id="bedrock-mantle-function",
    )
    results: list[JsonValue] = []
    for case in parsed["cases"]:
        config = RecordingMantlePassthrough()
        request_data: dict[str, JsonValue] = {"stream": False}
        _ = config.logging_non_streaming_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/passthrough/transformation.py#L53-L71
            model="fixture-model",
            custom_llm_provider=case["custom_llm_provider"],
            httpx_response=httpx.Response(200),
            request_data=request_data,
            logging_obj=logging_obj,
            endpoint=case["endpoint"],
        )
        results.append(config.forwarded[0])
    return {"results": results}
