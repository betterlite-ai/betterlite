from typing import cast

from litellm.llms.bedrock.chat.invoke_transformations.base_invoke_transformation import (
    AmazonInvokeConfig,
)

from .handlers import (
    base_options,
    guardrail_results,
    metadata_merge,
    prompts,
    signing_case,
    url_case,
)
from .models import InvokeApi, Json, Steps


def run(argument: Steps) -> Json:
    invoke = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L630
        "InvokeApi",
        AmazonInvokeConfig(),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/base_invoke_transformation.py#L78-L80
    )
    result: Json
    if "base_options" in argument:
        result = base_options(invoke, argument["base_options"])
    elif "provider_family_selection" in argument:
        models = argument["provider_family_selection"]["models"]
        result = {"providers": [invoke.get_bedrock_invoke_provider(model) for model in models]}
    elif "invoke_model_url" in argument:
        cases = argument["invoke_model_url"]["cases"]
        result = {"results": [url_case(invoke, case) for case in cases]}
    elif "guardrail_headers" in argument:
        result = guardrail_results(argument["guardrail_headers"]["values"])
    elif "guardrail_metadata_merge" in argument:
        result = metadata_merge(invoke, argument["guardrail_metadata_merge"])
    elif "signing_forwarding" in argument:
        result = signing_case(invoke, argument["signing_forwarding"])
    elif "prompt_template_selection" in argument:
        selection = argument["prompt_template_selection"]["cases"]
        result = prompts(invoke, selection, local_assets=True)
    elif "llama_local_template" in argument:
        llama = argument["llama_local_template"]
        result = prompts(invoke, llama["cases"], local_assets=llama.get("local_assets") is True)
    else:
        raise ValueError(f"unsupported step {sorted(argument)}")
    return result
