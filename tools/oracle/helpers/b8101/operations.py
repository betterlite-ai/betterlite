import httpx
from litellm.types.utils import ModelResponse

from .views import (
    FAILURE,
    MODEL,
    ConfigView,
    FlowInput,
    Json,
    ReplyCase,
    RequestCase,
    RequestsInput,
    SignCase,
    UrlCase,
    ValidateCase,
)


def url_value(config: ConfigView, case: UrlCase) -> Json:
    try:
        url = config.get_complete_url(
            api_base=case["api_base"],
            api_key=None,
            model=case["model"],
            optional_params=case["options"],
            litellm_params={},
            stream=None,
        )
    except FAILURE as error:
        return {"ok": False, "status": error.status_code}
    except ValueError:
        return {"ok": False, "status": None}
    return {"ok": True, "url": url}


def validate_value(config: ConfigView, case: ValidateCase) -> Json:
    return config.validate_environment(
        headers=case["headers"],
        model=MODEL,
        messages=[],
        optional_params={},
        litellm_params={},
        api_key=case["api_key"],
    )


def flow_value(config: ConfigView, input_value: FlowInput) -> Json:
    provider = input_value["provider"]
    base, key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/langflow/chat/transformation.py#L45-L54
        provider["api_base"],
        provider["api_key"],
    )
    return {
        "supported": config.get_supported_openai_params(model=MODEL),
        "provider": {"api_base": base, "api_key": key},
        "urls": [url_value(config, case) for case in input_value["urls"]],
        "validated": [
            validate_value(config, case) for case in input_value["validate"]
        ],
    }


def request_value(config: ConfigView, case: RequestCase) -> Json:
    try:
        body = config.transform_request(
            model=MODEL,
            messages=case["messages"],
            optional_params=case["options"],
            litellm_params={},
            headers={},
        )
    except FAILURE as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "body": body}


def requests_value(config: ConfigView, input_value: RequestsInput) -> Json:
    return {
        "bodies": [request_value(config, case) for case in input_value["requests"]],
        "signatures": [
            signature_value(config, case) for case in input_value["signatures"]
        ],
    }


def signature_value(config: ConfigView, case: SignCase) -> Json:
    try:
        headers, body = config.sign_request(
            headers=case["headers"],
            optional_params={},
            request_data=case["request_data"],
            api_base="",
            model=None,
            stream=None,
            fake_stream=None,
        )
    except FAILURE as error:
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "headers": headers, "body": body is not None}


def reply_value(config: ConfigView, case: ReplyCase) -> Json:
    try:
        response = config.transform_response(
            model=case["model"],
            raw_response=httpx.Response(
                case["status"],
                content=case["body"].encode(),
                request=httpx.Request("POST", "https://langflow.test"),
            ),
            model_response=ModelResponse(),
            logging_obj=None,
            request_data={},
            messages=case["messages"],
            optional_params={},
            litellm_params={},
            encoding=None,
        )
    except FAILURE as error:
        return {"ok": False, "status": error.status_code}
    choice = response.choices[0]
    return {
        "ok": True,
        "model": response.model,
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "role": choice.message.role,
        "content": choice.message.content,
    }
