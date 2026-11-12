from typing import NotRequired, TypeAlias, TypedDict

import litellm
from litellm import utils
from litellm.exceptions import UnsupportedParamsError
from litellm.utils import ProviderConfigManager

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]

LITELLM_UTILS = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py"


class Call(TypedDict):
    model: str
    custom_llm_provider: str
    user: NotRequired[str | None]
    encoding_format: NotRequired[str | None]
    dimensions: NotRequired[int | None]
    kwargs: Fields
    additional_drop_params: NotRequired[list[str] | None]
    allowed_openai_params: NotRequired[list[str] | None]
    drop_params: NotRequired[bool | None]
    litellm_drop_params: NotRequired[bool]
    force_branch: NotRequired[bool]


class Input(TypedDict):
    calls: list[Call]


class NoEmbeddingConfig:
    def __call__(self, *_args: Json, **_kwargs: Json) -> None:
        pass


NO_EMBEDDING_CONFIG = NoEmbeddingConfig()


def embedding_call(call: Call) -> Json:
    return utils.get_optional_params_embeddings(  # type: ignore[misc, no-any-return]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3747
        model=call["model"],
        user=call.get("user"),
        encoding_format=call.get("encoding_format"),
        dimensions=call.get("dimensions"),
        custom_llm_provider=call["custom_llm_provider"],
        drop_params=call.get("drop_params"),
        additional_drop_params=call.get("additional_drop_params"),
        allowed_openai_params=call.get("allowed_openai_params"),
        **call["kwargs"],
    )


def run(input_value: Input) -> Json:
    results: list[Json] = []
    original = ProviderConfigManager.get_provider_embedding_config
    for call in input_value["calls"]:
        litellm.drop_params = call.get("litellm_drop_params") is True
        if call.get("force_branch") is True:
            ProviderConfigManager.get_provider_embedding_config = NO_EMBEDDING_CONFIG  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8768
        try:
            results.append(embedding_call(call))
        except UnsupportedParamsError as failure:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
            error: dict[str, Json] = {"error": failure.status_code}
            results.append(error)
        finally:
            ProviderConfigManager.get_provider_embedding_config = original  # type: ignore[method-assign]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L8768
    return results
