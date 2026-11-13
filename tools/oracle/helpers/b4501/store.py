from typing import TypeVar

from litellm.proxy.lens.models import Job, Lens, Worker
from pydantic import JsonValue

from .model import (
    Seed,
    StoredLens,
    StoredRun,
    StoredWorker,
    Transform,
    count_row,
    dump,
    dump_mapping,
)

T = TypeVar("T")


def order_of(entry: tuple[str, str, JsonValue]) -> tuple[str, str]:
    created_at, job_id, _data = entry
    return (created_at, job_id)


class Store:
    def __init__(self, seed: Seed, conflicts: int) -> None:
        self.lenses: dict[str, StoredLens] = {}
        self.runs: dict[str, StoredRun] = {}
        self.workers: dict[str, StoredWorker] = {}
        self.conflicts = conflicts
        lenses = seed.get("lenses")
        if lenses is not None:
            for lens_id, row in lenses.items():
                lens = Lens.model_validate(row["data"])
                self.lenses[lens_id] = {"version": row["version"], "lens": lens}
        runs = seed.get("runs")
        if runs is not None:
            for job_id, row in runs.items():
                job = Job.model_validate(row["data"])
                self.runs[job_id] = {"lens_id": row["lens_id"], "job": job}
        workers = seed.get("workers")
        if workers is not None:
            for worker_id, row in workers.items():
                worker = Worker.model_validate(row["data"])
                self.workers[worker_id] = {
                    "token_hash": row["token_hash"],
                    "data": dump_mapping(worker),
                }

    async def query_raw(self, query: str, *args: T) -> list[dict[str, JsonValue]]:
        if query.startswith("WITH previous"):
            return self.rewrite(args)
        if "LiteLLM_LensWorker" in query:
            return self.worker_query(query, args)
        if "LiteLLM_LensRun" in query:
            return self.run_query(query, args)
        return self.lens_query(query, args)

    async def execute_raw(self, query: str, *args: T) -> int:
        if query.startswith('INSERT INTO "LiteLLM_Lens"'):
            lens = Lens.model_validate_json(str(args[1]))
            self.lenses[str(args[0])] = {"version": 0, "lens": lens}
            return 1
        if query.startswith('INSERT INTO "LiteLLM_LensWorker"'):
            worker = Worker.model_validate_json(str(args[2]))
            self.workers[str(args[0])] = {
                "token_hash": str(args[1]),
                "data": dump_mapping(worker),
            }
            return 1
        if query.startswith('UPDATE "LiteLLM_LensWorker" SET data=$1::jsonb'):
            worker = Worker.model_validate_json(str(args[0]))
            worker_id = str(args[1])
            if worker_id in self.workers:
                self.workers[worker_id]["data"] = dump_mapping(worker)
            return 1
        if "{revoked}" in query:
            row = self.workers.get(str(args[0]))
            if row is not None:
                row["data"]["revoked"] = True
            return 1
        if "{last_seen}" in query:
            row = self.workers.get(str(args[1]))
            if row is not None:
                row["data"]["last_seen"] = str(args[0])
            return 1
        return 0

    def lens_query(self, query: str, args: tuple[T, ...]) -> list[dict[str, JsonValue]]:
        if "ORDER BY id" in query:
            rows = [row for _, row in sorted(self.lenses.items())]
            return [{"data": dump(row["lens"])} for row in rows]
        row = self.lenses.get(str(args[0]))
        if row is None:
            return []
        return [{"data": dump(row["lens"])}]

    def rewrite(self, args: tuple[T, ...]) -> list[dict[str, JsonValue]]:
        data = str(args[0])
        lens_id = str(args[1])
        version = int(str(args[2]))
        if self.conflicts > 0:
            self.conflicts -= 1
            raced = self.lenses.get(lens_id)
            if raced is not None:
                next_version = raced["version"] + 1
                raced["version"] = next_version
                raced["lens"] = raced["lens"].model_copy(
                    update={"version": next_version}
                )
        row = self.lenses.get(lens_id)
        if row is None or row["version"] != version:
            return [count_row(0)]
        new_lens = Lens.model_validate_json(data)
        retained = {job.id for job in new_lens.jobs}
        for job in row["lens"].jobs:
            if job.id not in retained and job.id not in self.runs:
                self.runs[job.id] = {"lens_id": lens_id, "job": job}
        self.lenses[lens_id] = {"version": version + 1, "lens": new_lens}
        return [count_row(1)]

    def run_query(self, query: str, args: tuple[T, ...]) -> list[dict[str, JsonValue]]:
        lens_id = str(args[0])
        if "LIMIT 50" in query:
            offset = int(str(args[1]))
            entries: list[tuple[str, str, JsonValue]] = []
            for job_id, run in self.runs.items():
                if run["lens_id"] == lens_id:
                    entries.append(
                        (run["job"].created_at.isoformat(), job_id, dump(run["job"]))
                    )
            row = self.lenses.get(lens_id)
            if row is not None:
                entries.extend(
                    (job.created_at.isoformat(), job.id, dump(job))
                    for job in row["lens"].jobs
                )
            entries.sort(key=order_of, reverse=True)
            return [{"data": entry[2]} for entry in entries[offset : offset + 50]]
        job_id = str(args[1])
        run = self.runs.get(job_id)
        if run is not None and run["lens_id"] == lens_id:
            return [{"data": dump(run["job"])}]
        row = self.lenses.get(lens_id)
        if row is not None:
            for job in row["lens"].jobs:
                if job.id == job_id:
                    return [{"data": dump(job)}]
        return []

    def worker_query(self, query: str, args: tuple[T, ...]) -> list[dict[str, JsonValue]]:
        if "RETURNING" in query:
            row = self.workers.get(str(args[1]))
            if row is None or row["data"].get("revoked") is True:
                return []
            row["data"]["analysis_key_id"] = str(args[0])
            return [{"data": row["data"]}]
        if "token_hash=$1" in query:
            for row in self.workers.values():
                if row["token_hash"] == str(args[0]):
                    return [{"data": row["data"]}]
            return []
        return [{"data": row["data"]} for _, row in sorted(self.workers.items())]


def apply(lens: Lens, transform: Transform) -> Lens:
    kind = transform["kind"]
    if kind == "unchanged":
        return lens
    if kind == "set_revision":
        revision = transform.get("revision")
        if revision is None:
            raise ValueError("set_revision needs a revision")
        return lens.model_copy(update={"revision": revision})
    if kind == "drop_job":
        job_id = transform.get("job_id")
        if job_id is None:
            raise ValueError("drop_job needs a job_id")
        return lens.model_copy(
            update={"jobs": tuple(job for job in lens.jobs if job.id != job_id)}
        )
    raise RuntimeError("the transform failed")


class CountingTransform:
    def __init__(self, transform: Transform) -> None:
        self.transform = transform
        self.calls = 0

    def __call__(self, lens: Lens) -> Lens:
        self.calls += 1
        return apply(lens, self.transform)
