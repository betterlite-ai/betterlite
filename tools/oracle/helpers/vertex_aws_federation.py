from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.llms.vertex_ai.aws_credentials_supplier import AwsCredentialsSupplier
from litellm.llms.vertex_ai.vertex_ai_aws_wif import VertexAIAwsWifAuth

if TYPE_CHECKING:
    from collections.abc import Callable

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

EXTRACT_AWS_PARAMS = cast(
    "Callable[[dict[str, Json]], dict[str, str]]",
    VertexAIAwsWifAuth.extract_aws_params,
)
GET_AWS_REGION = cast(
    "Callable[[AwsCredentialsSupplier, object, object], str]",
    AwsCredentialsSupplier.get_aws_region,
)


def unused_credentials() -> None:
    return None


def run(data: dict[str, Json]) -> dict[str, object]:
    params = EXTRACT_AWS_PARAMS(dict(data))
    region = data.get("aws_region_name")
    aws_region: str | None = None
    if isinstance(region, str) and region:
        supplier = AwsCredentialsSupplier(
            credentials_provider=unused_credentials, aws_region=region
        )
        aws_region = GET_AWS_REGION(supplier, None, None)
    return {"aws_params": params, "aws_region": aws_region}
