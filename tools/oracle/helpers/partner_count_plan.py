from typing import Final, NotRequired, TypedDict

from litellm.llms.vertex_ai.vertex_ai_partner_models.count_tokens.handler import (
    VertexAIPartnerModelsTokenCounter,
)

COUNTER: Final = VertexAIPartnerModelsTokenCounter()
GET_PUBLISHER, BUILD_ENDPOINT, STRIP_SUFFIX = COUNTER._get_publisher_for_model, COUNTER._build_count_tokens_endpoint, COUNTER._strip_version_suffix  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_ai_partner_models/count_tokens/handler.py#L24-L93


class PlanCase(TypedDict):
    model: str
    project: str
    location: str
    api_base: NotRequired[str | None]


class PlanRow(TypedDict):
    publisher: str
    endpoint: str
    stripped: str


class Input(TypedDict):
    cases: list[PlanCase]


class Output(TypedDict):
    cases: list[PlanRow]


def run(input_value: Input) -> Output:
    rows: list[PlanRow] = []
    for case in input_value["cases"]:
        publisher = GET_PUBLISHER(case["model"])
        endpoint = BUILD_ENDPOINT(
            model=case["model"],
            project_id=case["project"],
            vertex_location=case["location"],
            api_base=case.get("api_base"),
        )
        rows.append(
            {
                "publisher": publisher,
                "endpoint": endpoint,
                "stripped": STRIP_SUFFIX(case["model"]),
            }
        )
    return {"cases": rows}
