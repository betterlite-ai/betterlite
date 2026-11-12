from typing import NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.llms.bedrock.base_aws_llm import BaseAWSLLM
from litellm.llms.bedrock.common_utils import bedrock_route_for_request
from pydantic import ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Settings(TypedDict, total=False):
    aws_region_name: str | None
    aws_region: str | None
    session_region: str | None
    aws_bedrock_runtime_endpoint: str | None


class PlanCase(TypedDict):
    model: str
    model_id: NotRequired[str | None]
    aws_region_name: NotRequired[str | None]
    api_base: NotRequired[str | None]
    aws_bedrock_runtime_endpoint: NotRequired[str | None]
    settings: NotRequired[Settings]
    request_params: NotRequired[dict[str, Json]]
    additional_drop_params: NotRequired[list[str]]


class Input(TypedDict):
    cases: list[PlanCase]
    price_map: NotRequired[dict[str, Json]]
    converse_models: NotRequired[list[str]]
    request_metadata_fields: NotRequired[list[str]]


def apply_catalogs(input_value: Input) -> None:
    costs = cast("dict[str, Json]", litellm.model_cost)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    costs.clear()
    price_map = input_value.get("price_map")
    costs.update(price_map if price_map is not None else {})
    converse = cast("set[str]", litellm.bedrock_converse_models)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L636
    converse.clear()
    converse_models = input_value.get("converse_models")
    converse.update(converse_models if converse_models is not None else [])
    litellm.bedrock_request_metadata_fields = input_value.get("request_metadata_fields")


def optional_params(case: PlanCase) -> dict[str, Json]:
    params: dict[str, Json] = {}
    model_id = case.get("model_id")
    if model_id is not None:
        params["model_id"] = model_id
    region = case.get("aws_region_name")
    if region is not None:
        params["aws_region_name"] = region
    return params


def plan_case(case: PlanCase) -> Json:
    model = case["model"]
    params_value = case.get("request_params")
    request_params: dict[str, Json] = params_value if params_value is not None else {}
    drops_value = case.get("additional_drop_params")
    drops = drops_value if drops_value is not None else []
    try:
        route = bedrock_route_for_request(model, request_params, drops)
    except ValidationError:
        return {"error": "route"}
    if route != "invoke":
        return {"plan": {"operation": route, "model": model}}
    params = optional_params(case)
    provider = BaseAWSLLM.get_bedrock_invoke_provider(model)
    model_id = BaseAWSLLM.get_bedrock_model_id(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L618
        params, provider, model
    )
    instance = BaseAWSLLM()
    try:
        region = instance._get_aws_region_name(params, model=model)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/base_aws_llm.py#L732
    except ValueError:
        return {"error": "region"}
    endpoint_url, proxy_endpoint_url = instance.get_runtime_endpoint(
        api_base=case.get("api_base"),
        aws_bedrock_runtime_endpoint=case.get("aws_bedrock_runtime_endpoint"),
        aws_region_name=region,
    )
    plan: dict[str, Json] = {
        "operation": "invoke",
        "model": model,
        "model_id": model_id,
        "region": region,
        "endpoint_url": endpoint_url,
        "proxy_endpoint_url": proxy_endpoint_url,
    }
    output: dict[str, Json] = {"plan": plan}
    return output


def run(input_value: Input) -> Json:
    apply_catalogs(input_value)
    plans: list[Json] = [plan_case(case) for case in input_value["cases"]]
    output: dict[str, Json] = {"plans": plans}
    return output
