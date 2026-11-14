import asyncio

from litellm.proxy.lens.models import Worker
from litellm.proxy.lens.repository import LensRepository
from pydantic import JsonValue

from .model import Call, Output, dump
from .store import CountingTransform, Store


def read_call(repository: LensRepository, call: Call) -> tuple[JsonValue, int] | None:
    operation = call["operation"]
    if operation == "jobs":
        lens_id = call.get("lens_id")
        offset = call.get("offset")
        if lens_id is None or offset is None:
            raise ValueError("jobs needs lens_id and offset")
        return ([dump(job) for job in asyncio.run(repository.jobs(lens_id, offset))], 0)
    if operation == "job":
        lens_id = call.get("lens_id")
        job_id = call.get("job_id")
        if lens_id is None or job_id is None:
            raise ValueError("job needs lens_id and job_id")
        found = asyncio.run(repository.job(lens_id, job_id))
        return (None if found is None else dump(found), 0)
    return None


def update_call(repository: LensRepository, call: Call) -> tuple[JsonValue, int]:
    lens_id = call.get("lens_id")
    attempts = call.get("attempts")
    changed_only = call.get("changed_only")
    transform = call.get("transform")
    if lens_id is None or attempts is None or changed_only is None or transform is None:
        raise ValueError("update needs lens_id, attempts, changed_only and transform")
    counting = CountingTransform(transform)
    updated = asyncio.run(repository.update(lens_id, counting, attempts, changed_only=changed_only))
    return (None if updated is None else dump(updated), counting.calls)


def worker_call(repository: LensRepository, call: Call) -> JsonValue:
    operation = call["operation"]
    if operation == "save_worker":
        payload = call.get("worker")
        if payload is None:
            raise ValueError("save_worker needs a worker")
        token_hash = call.get("token_hash")
        asyncio.run(repository.save_worker(Worker.model_validate(payload), token_hash))
        return None
    if operation == "set_worker_billing":
        worker_id = call.get("worker_id")
        key_id = call.get("key_id")
        if worker_id is None or key_id is None:
            raise ValueError("set_worker_billing needs worker_id and key_id")
        updated = asyncio.run(repository.set_worker_billing(worker_id, key_id))
        return None if updated is None else dump(updated)
    if operation == "revoke_worker":
        worker_id = call.get("worker_id")
        if worker_id is None:
            raise ValueError("revoke_worker needs worker_id")
        asyncio.run(repository.revoke_worker(worker_id))
        return None
    worker_id = call.get("worker_id")
    now = call.get("now")
    if worker_id is None or now is None:
        raise ValueError("heartbeat needs worker_id and now")
    asyncio.run(repository.heartbeat(worker_id, now))
    return None


def output(store: Store, result: JsonValue, transform_calls: int) -> Output:
    lenses: list[dict[str, JsonValue]] = []
    for lens_id, lens_row in sorted(store.lenses.items()):
        lenses.append(
            {"id": lens_id, "version": lens_row["version"], "data": dump(lens_row["lens"])}
        )
    runs: list[dict[str, JsonValue]] = []
    for job_id, run_row in sorted(store.runs.items()):
        runs.append({"id": job_id, "lens_id": run_row["lens_id"], "data": dump(run_row["job"])})
    workers: list[dict[str, JsonValue]] = []
    for worker_id, worker_row in sorted(store.workers.items()):
        workers.append(
            {"id": worker_id, "token_hash": worker_row["token_hash"], "data": worker_row["data"]}
        )
    return {
        "result": result,
        "transform_calls": transform_calls,
        "lenses": lenses,
        "runs": runs,
        "workers": workers,
    }
