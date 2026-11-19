from typing import cast

from litellm.llms.bedrock_mantle.messages.transformation import (
    BedrockMantleAnthropicMessagesConfig,
)
from pydantic import JsonValue

from .support import MESSAGE_PAYLOAD, environment, region_params
from .views import MessagesApi


def message_cases(payload: JsonValue) -> JsonValue:
    parsed = MESSAGE_PAYLOAD.validate_python(payload)
    config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/messages/transformation.py#L39-L67
        "MessagesApi", BedrockMantleAnthropicMessagesConfig()
    )
    urls: list[JsonValue] = []
    for case in parsed["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        with environment(values):
            try:
                url = config.get_complete_url(
                    api_base=case.get("api_base"),
                    api_key=None,
                    model="",
                    optional_params={},
                    litellm_params=region_params(case.get("aws_region_name")),
                    stream=None,
                )
                urls.append({"url": url})
            except ValueError:
                urls.append({"error": True})
    return {
        "provider": config.custom_llm_provider,
        "uses_get_llm_provider_api_base": config.uses_get_llm_provider_api_base(),
        "urls": urls,
    }
