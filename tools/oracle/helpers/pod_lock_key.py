from typing import TypedDict

from litellm.proxy.db.db_transaction_queue.pod_lock_manager import PodLockManager


class Input(TypedDict):
    cronjob_id: str


def run(input_value: Input) -> str:
    return PodLockManager.get_redis_lock_key(input_value["cronjob_id"])
