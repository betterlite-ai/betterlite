import copy
from base64 import b64encode

import litellm
from litellm.constants import known_tokenizer_config
from litellm.llms.bedrock.common_utils import BedrockError

from .models import (
    BaseOptions,
    InvokeApi,
    Json,
    MetadataMerge,
    MetadataMergeCase,
    Params,
    PromptCase,
    SigningCase,
    UrlCase,
)
from .stubs import (
    FixedCredentials,
    guardrail_headers,
    sealed_environment,
    signing_harness,
    unreachable_templates,
)

BUILTINS = copy.deepcopy(known_tokenizer_config)


def base_options(invoke: InvokeApi, case: BaseOptions) -> Json:
    optional = dict(case["optional"])
    mapped = invoke.map_openai_params(dict(case["non_default"]), optional, "", drop_params=False)
    invoke._apply_config_to_params(dict(case["config"]), mapped)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L169-L173
    supported: list[Json] = list(invoke.get_supported_openai_params(""))
    return {"supported": supported, "mapped": mapped}


def url_case(invoke: InvokeApi, case: UrlCase) -> Json:
    optional = case.get("optional_params")
    params: Params = {} if optional is None else dict(optional)
    environment = case.get("environment")
    model = case["model"]
    with sealed_environment({} if environment is None else dict(environment)):
        try:
            provider = invoke.get_bedrock_invoke_provider(model)
            model_id = invoke.get_bedrock_model_id(dict(params), provider, model)
            api_base = case.get("api_base")
            stream = case.get("stream")
            endpoint = invoke.get_complete_url(api_base, None, model, params, {}, stream=stream)
            region = invoke._get_aws_region_name(params, model)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L732
        except (TypeError, ValueError):
            return {"error": True}
    return {
        "endpoint_url": endpoint,
        "model_id": model_id,
        "provider": provider,
        "region": region,
    }


def guardrail_results(values: list[Json]) -> Json:
    results: list[Json] = []
    for value in values:
        try:
            derived: dict[str, Json] = dict(guardrail_headers(value))
        except BedrockError as error:
            results.append({"status": error.status_code})
            continue
        results.append({"headers": derived})
    return {"results": results}


def merge_case(invoke: InvokeApi, case: MetadataMergeCase) -> Json:
    headers = case.get("headers")
    optional = case.get("optional_params")
    identity = case.get("identity")
    spend = case.get("spend_logs_metadata")
    policy_fields = case.get("policy_fields")
    bucket: Params = {} if identity is None else dict(identity)
    if spend:
        bucket["spend_logs_metadata"] = dict(spend)
    original_fields = litellm.bedrock_request_metadata_fields
    litellm.bedrock_request_metadata_fields = [] if policy_fields is None else list(policy_fields)
    try:
        merged: dict[str, Json] = dict(
            invoke.validate_environment(
                {} if headers is None else dict(headers),
                "",
                [],
                {} if optional is None else dict(optional),
                {"metadata": bucket},
            )
        )
    except BedrockError:
        return {"error": True}
    finally:
        litellm.bedrock_request_metadata_fields = original_fields
    return {"headers": merged}


def metadata_merge(invoke: InvokeApi, case: MetadataMerge) -> Json:
    error = case["error"]
    supplied = error.get("headers")
    projected = invoke.get_error_class(
        error["message"], error["status_code"], {} if supplied is None else dict(supplied)
    )
    headers: dict[str, Json] = {} if projected.headers is None else dict(projected.headers)
    projection: dict[str, Json] = {
        "status": projected.status_code,
        "message": projected.message,
        "headers": headers,
    }
    return {"merges": [merge_case(invoke, value) for value in case["merges"]], "error": projection}


def signing_case(invoke: InvokeApi, case: SigningCase) -> Json:
    headers = case.get("headers")
    optional = case.get("optional_params")
    request = case.get("request_data")
    credentials = FixedCredentials(case["credentials"])
    with signing_harness(case.get("configured_bearer_token"), credentials, case["now"]):
        signed_headers, body = invoke.sign_request(
            headers={} if headers is None else dict(headers),
            optional_params={} if optional is None else dict(optional),
            request_data={} if request is None else dict(request),
            api_base=case["api_base"],
            api_key=case.get("api_key"),
            model=case.get("model"),
            stream=case.get("stream"),
            fake_stream=case.get("fake_stream"),
        )
    signed: dict[str, Json] = dict(signed_headers)
    body_text = b64encode(b"" if body is None else body).decode()
    return {"method": "POST", "url": case["api_base"], "headers": signed, "body": body_text}


def prompts(invoke: InvokeApi, cases: list[PromptCase], *, local_assets: bool) -> Json:
    results: list[Json] = []
    for case in cases:
        known_tokenizer_config.clear()
        if local_assets:
            known_tokenizer_config.update(copy.deepcopy(BUILTINS))
        custom = case.get("custom_prompt_dict")
        custom_prompt_dict = {} if custom is None else dict(custom)
        with unreachable_templates():
            try:
                prompt, history = invoke.convert_messages_to_prompt(
                    case["model"], case["messages"], case.get("provider"), custom_prompt_dict
                )
            except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L592-L630
                results.append({"message": str(error)})
                continue
        results.append({"prompt": prompt, "chat_history": history})
    return {"results": results}
