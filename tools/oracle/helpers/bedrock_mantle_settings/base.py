from typing import cast
from unittest.mock import patch

import litellm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock_mantle.chat.transformation import BedrockMantleChatConfig
from litellm.llms.bedrock_mantle.common_utils import resolve_mantle_region
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue

from .models import BASE_PAYLOAD, BaseCase
from .support import environment
from .views import ChatApi


def base_result(config: ChatApi, case: BaseCase) -> tuple[JsonValue, JsonValue]:
    params = GenericLiteLLMParams(aws_region_name=case.get("aws_region_name"))
    try:
        api_base, api_key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L58-L84
            case.get("api_base"),
            case.get("api_key"),
            params,
            case.get("model"),
        )
        chat: JsonValue = {"api_base": api_base, "api_key": api_key}
    except ValueError:
        chat = {"error": True}
    mapping: dict[str, JsonValue] = {
        "aws_region_name": case.get("aws_region_name"),
        "api_base": case.get("api_base"),
    }
    try:
        standalone: JsonValue = {"region": resolve_mantle_region(mapping)}
    except ValueError:
        standalone = {"error": True}
    return chat, standalone


def base_cases(payload: JsonValue) -> JsonValue:
    parsed = BASE_PAYLOAD.validate_python(payload)
    quoted = parsed.get("model_cost")
    model_cost: dict[str, JsonValue] = {} if quoted is None else quoted
    signers: list[JsonValue] = []
    outcomes: list[tuple[JsonValue, JsonValue]] = []
    for case in parsed["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        signer = BaseAWSLLM()
        with (
            environment(values),
            patch.object(litellm, "model_cost", model_cost),
        ):
            config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L41-L43
                "ChatApi",
                BedrockMantleChatConfig(
                    aws_signer=signer if case.get("inject_signer") is True else None
                ),
            )
            kept = config._aws_signer is signer  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L41-L43
            signers.append("injected" if kept else "default")
            outcomes.append(base_result(config, case))
    default = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L46-L51
        "ChatApi", BedrockMantleChatConfig()
    )
    return {
        "provider": default.custom_llm_provider,
        "get_config": default.get_config(),
        "signers": signers,
        "chat": [chat for chat, _region in outcomes],
        "standalone": [region for _chat, region in outcomes],
    }
