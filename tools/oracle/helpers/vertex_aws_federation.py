from collections.abc import Callable
from typing import TypeAlias, TypedDict, cast

from litellm.llms.vertex_ai.aws_credentials_supplier import AwsCredentialsSupplier
from litellm.llms.vertex_ai.vertex_ai_aws_wif import VertexAIAwsWifAuth

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Output(TypedDict):
    aws_params: dict[str, str]
    aws_region: str | None


def run(data: dict[str, Json]) -> Output:
    params = VertexAIAwsWifAuth.extract_aws_params(data)
    aws_region: str | None = None
    match data.get("aws_region_name"):
        case str() as region if region:
            supplier = AwsCredentialsSupplier(
                credentials_provider=dict, aws_region=region
            )
            get_aws_region = cast(Callable[[None, None], str], supplier.get_aws_region)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/aws_credentials_supplier.py#L51
            aws_region = get_aws_region(None, None)
        case str() | None | bool() | int() | float() | list() | dict():
            pass
    return {"aws_params": params, "aws_region": aws_region}
