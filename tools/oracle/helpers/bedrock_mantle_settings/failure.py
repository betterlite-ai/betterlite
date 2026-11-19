from typing import cast

from litellm.llms.bedrock_mantle.chat.transformation import BedrockMantleChatConfig
from pydantic import JsonValue

from .models import FAILURE_PAYLOAD, FailureKind
from .support import (
    AUTH_FAILURE_MESSAGE,
    AUTH_MODULE,
    EXCEPTIONS,
    FailingCredentials,
    FrozenClock,
    environment,
    region_params,
)
from .views import ChatApi


def credential_failure(kind: FailureKind) -> Exception:
    if kind == "partial_credentials":
        return EXCEPTIONS.PartialCredentialsError(provider="fixture", cred_var="fixture-credential")
    if kind == "profile_not_found":
        return EXCEPTIONS.ProfileNotFound(profile="fixture")
    if kind == "credential_retrieval":
        return EXCEPTIONS.CredentialRetrievalError(provider="fixture", error_msg="fixture failure")
    return EXCEPTIONS.NoCredentialsError()


def failure_cases(payload: JsonValue) -> JsonValue:
    parsed = FAILURE_PAYLOAD.validate_python(payload)
    results: list[JsonValue] = []
    for case in parsed["cases"]:
        env = case.get("environment")
        values = {} if env is None else env
        given = case.get("api_base")
        api_base = "https://bedrock-mantle.us-east-1.api.aws/v1" if given is None else given
        config = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock_mantle/chat/transformation.py#L36-L143
            "ChatApi",
            BedrockMantleChatConfig(aws_signer=FailingCredentials(credential_failure(case["kind"]))),
        )
        with environment(values):
            AUTH_MODULE.get_current_datetime = FrozenClock(1_700_000_000)
            try:
                _ = config.sign_request(
                    {},
                    region_params(case.get("aws_region_name")),
                    {"messages": [{}]},
                    api_base,
                    api_key=None,
                    model="openai.gpt-oss-120b",
                )
            except ValueError as error:
                message = str(error)
                if message == AUTH_FAILURE_MESSAGE:
                    results.append({"kind": "auth_failure", "message": message})
                else:
                    results.append({"kind": "region_error"})
            else:
                results.append({"kind": "signed"})
    return {"results": results}
