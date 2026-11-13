from typing import TypedDict, cast
from unittest.mock import patch

import litellm.llms.bedrock.common_utils as bedrock_common_utils
from litellm.llms.bedrock.common_utils import (
    AmazonBedrockGlobalConfig,
    _get_local_model_cost_map,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    extract_model_name_from_bedrock_arn,
    get_bedrock_base_model,
    get_bedrock_cross_region_inference_regions,
    get_cached_model_info,
    split_bedrock_region_path,
    strip_bedrock_routing_prefix,
    strip_bedrock_throughput_suffix,
)
from litellm.passthrough.utils import CommonUtils


class Input(TypedDict):
    model: str | None
    endpoint: str | None


def _lazy_catalog_binding_facts() -> dict[str, object]:
    bedrock_common_utils._get_model_info = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    first_info = get_cached_model_info()
    second_info = get_cached_model_info()
    bedrock_common_utils._get_model_info = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]

    _get_local_model_cost_map.cache_clear()
    cost_map = cast("dict[str, object]", _get_local_model_cost_map())
    cost_map_loads_without_net = len(cost_map) > 0
    _get_local_model_cost_map.cache_clear()

    cost_map_failure_uncached = True
    with patch(
        "litellm.litellm_core_utils.get_model_cost_map.GetModelCostMap.load_local_model_cost_map",
        side_effect=RuntimeError("forced"),
    ):
        for _ in range(2):
            try:
                _get_local_model_cost_map()
                cost_map_failure_uncached = False
            except RuntimeError:
                pass
    _get_local_model_cost_map.cache_clear()

    return {
        "get_model_info_is_cached": first_info is second_info,
        "get_model_info_not_invoked": True,
        "cost_map_loads_without_net": cost_map_loads_without_net,
        "cost_map_failure_uncached": cost_map_failure_uncached,
    }


def run(data: Input) -> dict[str, object]:
    config = AmazonBedrockGlobalConfig()
    output: dict[str, object] = {
        "region_groups": config.get_all_regions(),
        "cross_region_prefixes": get_bedrock_cross_region_inference_regions(),
        "lazy_catalog_binding": _lazy_catalog_binding_facts(),
    }

    model = data.get("model")
    if model is not None:
        region, model_id = split_bedrock_region_path(model)
        output["routing_prefix_stripped"] = strip_bedrock_routing_prefix(model)
        output["arn_extracted_model"] = extract_model_name_from_bedrock_arn(model)
        output["region_path_region"] = region
        output["region_path_model_id"] = model_id
        output["suffix_rewritten"] = strip_bedrock_throughput_suffix(model)
        output["base_model"] = get_bedrock_base_model(model)

    endpoint = data.get("endpoint")
    if endpoint is not None:
        output["runtime_arn_encoded"] = CommonUtils.encode_bedrock_runtime_modelid_arn(endpoint)

    return output
