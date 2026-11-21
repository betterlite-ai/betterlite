from datetime import date, datetime
from typing import Literal, TypedDict

from litellm.proxy.db.db_transaction_queue.spend_logs_partition_manager import (
    parse_partition_upper_bound,
    select_partitions_to_drop,
    upcoming_partitions,
)


class PartitionInput(TypedDict):
    name: str
    bound: str


class Input(TypedDict):
    function: Literal["upcoming", "drop"]
    today: str
    interval: Literal["day", "week", "month"]
    ahead: int
    cutoff: str
    partitions: list[PartitionInput]


def run(input_value: Input) -> list[list[str]] | list[str]:
    if input_value["function"] == "upcoming":
        specs = upcoming_partitions(
            date.fromisoformat(input_value["today"]),
            input_value["interval"],
            input_value["ahead"],
        )
        return [
            [name, start.isoformat(), upper.isoformat()] for name, start, upper in specs
        ]
    partitions = [
        (partition["name"], parse_partition_upper_bound(partition["bound"]))
        for partition in input_value["partitions"]
    ]
    return select_partitions_to_drop(
        partitions, datetime.fromisoformat(input_value["cutoff"])
    )
