from base64 import b64encode
from typing import Final

import litellm
from pydantic import JsonValue, TypeAdapter

from bedrock_invoke_primitives.stubs import FixedCredentials, sealed_environment, signing_harness

from .views import InvokeUrl, Object, Probe

OBJECT: Final[TypeAdapter[Object]] = TypeAdapter(Object)


def metadata_bucket(call: InvokeUrl) -> Object:
    bucket: Object = {}
    if "metadata" in call:
        bucket.update(call["metadata"])
    if "spend_logs_metadata" in call:
        bucket["spend_logs_metadata"] = call["spend_logs_metadata"]
    return bucket


def invoke_environment(call: InvokeUrl) -> tuple[dict[str, str], str | None]:
    original = litellm.bedrock_request_metadata_fields
    fields = call.get("policy_fields")
    litellm.bedrock_request_metadata_fields = [] if fields is None else list(fields)
    headers = call.get("headers")
    optional = call.get("optional_params")
    try:
        return Probe().view().validate_anthropic_messages_environment(
            headers={} if headers is None else headers,
            model=call["model"],
            messages=[],
            optional_params={} if optional is None else optional,
            litellm_params={"metadata": metadata_bucket(call)},
            api_key=call.get("api_key"),
            api_base=call.get("api_base"),
        )
    finally:
        litellm.bedrock_request_metadata_fields = original


def signed_request(call: InvokeUrl, url: str, headers: dict[str, str]) -> JsonValue:
    optional = call.get("optional_params")
    request_data = call.get("request_data")
    with signing_harness(
        call.get("configured_bearer_token"), FixedCredentials(call["credentials"]), call["now"]
    ):
        signed_headers, body = Probe().view().sign_request(
            headers=headers,
            optional_params={} if optional is None else optional,
            request_data={} if request_data is None else request_data,
            api_base=url,
            api_key=call.get("api_key"),
            model=call["model"],
            stream=call.get("stream"),
            fake_stream=call.get("fake_stream"),
        )
    return {
        "method": "POST",
        "url": url,
        "headers": OBJECT.validate_python(signed_headers),
        "body": b64encode(b"" if body is None else body).decode(),
    }


def complete_url(call: InvokeUrl) -> str:
    environment = call.get("environment")
    optional = call.get("optional_params")
    with sealed_environment({} if environment is None else environment):
        return Probe().view().get_complete_url(
            api_base=call.get("api_base"),
            api_key=call.get("api_key"),
            model=call["model"],
            optional_params={} if optional is None else optional,
            litellm_params={},
            stream=call.get("stream"),
        )


def invoke_url(call: InvokeUrl) -> JsonValue:
    headers, api_base = invoke_environment(call)
    url = complete_url(call)
    return {
        "url": {"endpoint_url": url},
        "environment": {"headers": OBJECT.validate_python(headers), "api_base": api_base},
        "signed": signed_request(call, url, headers),
    }
