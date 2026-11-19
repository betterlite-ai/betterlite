from collections.abc import Mapping
from typing import cast

from litellm.llms.bedrock_mantle.chat.transformation import BedrockMantleChatConfig
from pydantic import JsonValue

from .support import AUTH_MODULE, JSON, SIGN_PAYLOAD, FixedCredentials, FrozenClock, environment
from .views import ChatApi


def signed_value(api_base: str, headers: Mapping[str, str | None], body: bytes | None) -> JsonValue:
    projected: dict[str, JsonValue] = {}
    authorization: JsonValue = None
    for name, value in headers.items():
        if value is None:
            continue
        if name.lower() == "authorization":
            authorization = value if value.startswith("Bearer ") else value.partition(",")[0]
        else:
            projected[name.lower()] = value
    parsed: JsonValue = None if body is None else JSON.validate_json(body)
    return {
        "url": api_base,
        "method": "POST",
        "headers": projected,
        "body": parsed,
        "authorization": authorization,
    }


def sign_cases(payload: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for case in SIGN_PAYLOAD.validate_python(payload)["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        headers = case.get("headers")
        optional_params = case.get("optional_params")
        request_data = case.get("request_data")
        signed_headers: dict[str, str | None] = {}
        body: bytes | None = None
        config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L36-L143
            "ChatApi",
            BedrockMantleChatConfig(aws_signer=FixedCredentials(case["credentials"])),
        )
        with environment(values):
            AUTH_MODULE.get_current_datetime = FrozenClock(case["now"])
            try:
                signed_headers, body = config.sign_request(
                    {} if headers is None else dict(headers),
                    {} if optional_params is None else optional_params,
                    {} if request_data is None else request_data,
                    case["api_base"],
                    api_key=case.get("api_key"),
                    model=case.get("model"),
                    stream=case.get("stream"),
                    fake_stream=case.get("fake_stream"),
                )
            except ValueError:
                results.append({"error": True})
                continue
        results.append(signed_value(case["api_base"], signed_headers, body))
    return {"results": results}
