from typing import cast

from litellm.llms.bedrock_mantle.passthrough.transformation import (
    BedrockMantlePassthroughConfig,
)
from pydantic import JsonValue

from .support import PASSTHROUGH_PAYLOAD, PassthroughCase, environment, region_params
from .views import PassthroughApi


def passthrough_result(
    config: PassthroughApi, case: PassthroughCase, params: dict[str, JsonValue]
) -> JsonValue:
    try:
        region = config._get_aws_region_name(params, "ignored-model", "ignored-id")  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/passthrough/transformation.py#L26-L32
    except ValueError:
        return {"error": True}
    endpoint_type = case.get("endpoint_type")
    endpoint = config.get_runtime_endpoint(
        case.get("api_base"),
        case.get("aws_bedrock_runtime_endpoint"),
        region,
        "runtime" if endpoint_type is None else endpoint_type,
    )
    litellm_params: dict[str, JsonValue] = {}
    key = case.get("api_key")
    if key is not None:
        litellm_params["api_key"] = key
    bearer = config.get_bedrock_bearer_token(litellm_params)
    return {"region": region, "endpoint": [endpoint[0], endpoint[1]], "bearer": bearer}


def passthrough_cases(payload: JsonValue) -> JsonValue:
    parsed = PASSTHROUGH_PAYLOAD.validate_python(payload)
    config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/passthrough/transformation.py#L19-L71
        "PassthroughApi", BedrockMantlePassthroughConfig()
    )
    results: list[JsonValue] = []
    for case in parsed["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        params = region_params(case.get("aws_region_name"))
        base = case.get("api_base")
        if base is not None:
            params["api_base"] = base
        with environment(values):
            results.append(passthrough_result(config, case, params))
    return {"results": results}
