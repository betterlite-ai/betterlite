from typing import Literal, TypedDict

from litellm_proxy_extras.request_log_indexes import RequestLogIndex


class Input(TypedDict):
    function: Literal["columns", "partition_index_name"]
    table: str
    name: str
    definition: str
    partition: str


def run(input_value: Input) -> tuple[str, ...] | str:
    index = RequestLogIndex(
        table=input_value["table"],
        name=input_value["name"],
        definition=input_value["definition"],
    )
    if input_value["function"] == "columns":
        return index.columns
    return index.partition_index_name(input_value["partition"])
