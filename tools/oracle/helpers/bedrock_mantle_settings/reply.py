from datetime import UTC, datetime
from typing import cast
from unittest.mock import patch

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.passthrough.transformation import BedrockPassthroughConfig
from litellm.llms.bedrock_mantle.passthrough.transformation import (
    BedrockMantlePassthroughConfig,
)
from pydantic import JsonValue

from .models import REPLY_PAYLOAD
from .support import JSON
from .views import ReplyApi


class ReplyRecorder:
    def __init__(self) -> None:
        self.seen: list[JsonValue] = []

    def __call__(self, **kwargs: JsonValue) -> JsonValue:
        record: dict[str, JsonValue] = {
            "model": kwargs["model"],
            "provider": kwargs["custom_llm_provider"],
            "endpoint": kwargs["endpoint"],
            "request_data": kwargs["request_data"],
        }
        self.seen.append(record)
        return record


def reply_cases(payload: JsonValue) -> JsonValue:
    parsed = REPLY_PAYLOAD.validate_python(payload)
    config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/passthrough/transformation.py#L19-L71
        "ReplyApi", BedrockMantlePassthroughConfig()
    )
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
        recorder = ReplyRecorder()
        with patch.object(BedrockPassthroughConfig, "logging_non_streaming_response", recorder):
            value = config.logging_non_streaming_response(
                model="fixture-model",
                custom_llm_provider=case["custom_llm_provider"],
                httpx_response=httpx.Response(200),
                request_data={"stream": False},
                logging_obj=logging_obj,
                endpoint=case["endpoint"],
            )
        results.append(JSON.validate_python(value))
    return {"results": results}
