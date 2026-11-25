import tempfile
from collections.abc import Callable, Iterable, Sequence
from pathlib import Path
from types import ModuleType
from typing import Literal, Protocol, TypedDict

from litellm.integrations.prometheus_metrics_endpoint import (
    parse_scrape_request,
    render_scrape,
)
from prometheus_client import CollectorRegistry
from prometheus_client.metrics_core import Metric
from prometheus_client.mmap_dict import MmapedDict, mmap_key
from prometheus_client.multiprocess import MultiProcessCollector
from prometheus_client.samples import Sample


class EntrySpec(TypedDict):
    metric: str
    sample: str
    labels: dict[str, str]
    help: str
    value: float
    timestamp: float


class FileSpec(TypedDict):
    name: str
    entries: list[EntrySpec]


class Input(TypedDict):
    files: list[FileSpec]
    call: Literal["merge", "scrape"]
    accept: str


class SampleOut(TypedDict):
    name: str
    labels: dict[str, str]
    value: float


class FamilyOut(TypedDict):
    name: str
    help: str
    kind: str
    samples: list[SampleOut]


class MergeAnswer(TypedDict):
    files: list[str]
    families: list[FamilyOut]


class ScrapeAnswer(TypedDict):
    files: list[str]
    blocks: list[str]


class Mmap(Protocol):
    def write_value(self, key: str, value: float, timestamp: float) -> None: ...

    def close(self) -> None: ...


class Collector(Protocol):
    def merge(self, files: Sequence[str], *, accumulate: bool) -> Iterable[Metric]: ...


class PrometheusModule(ModuleType):
    MmapedDict: Callable[[str], Mmap]
    MultiProcessCollector: Callable[[CollectorRegistry, str], Collector]


def prometheus_module() -> PrometheusModule:
    module = PrometheusModule("prometheus_client")
    module.MmapedDict = MmapedDict
    module.MultiProcessCollector = MultiProcessCollector
    return module


def write_files(directory: Path, input_value: Input) -> list[Path]:
    module = prometheus_module()
    paths: list[Path] = []
    for spec in input_value["files"]:
        path = directory / spec["name"]
        mmap = module.MmapedDict(str(path))
        for entry in spec["entries"]:
            labels = entry["labels"]
            mmap.write_value(
                mmap_key(
                    entry["metric"],
                    entry["sample"],
                    list(labels.keys()),
                    list(labels.values()),
                    entry["help"],
                ),
                entry["value"],
                entry["timestamp"],
            )
        mmap.close()
        paths.append(path)
    return paths


def used_hex(path: Path) -> str:
    data = path.read_bytes()
    used = int.from_bytes(data[:4], "little", signed=True)
    return data[: max(used, 0)].hex()


def sample_key(sample: SampleOut) -> str:
    labels = sample["labels"]
    return mmap_key(sample["name"], "", list(labels.keys()), list(labels.values()), "")


def samples_out(samples: Sequence[Sample]) -> list[SampleOut]:
    rendered: list[SampleOut] = [
        {"name": sample.name, "labels": dict(sample.labels), "value": sample.value}
        for sample in samples
    ]
    rendered.sort(key=sample_key)
    return rendered


def family_out(metric: Metric) -> FamilyOut:
    return {
        "name": metric.name,
        "help": metric.documentation,
        "kind": metric.type,
        "samples": samples_out(metric.samples),
    }


def families_out(metrics: Iterable[Metric]) -> list[FamilyOut]:
    families: list[FamilyOut] = [family_out(metric) for metric in metrics]
    indexed: list[tuple[str, int]] = [
        (family["name"], index) for index, family in enumerate(families)
    ]
    indexed.sort()
    return [families[index] for _, index in indexed]


def blocks(text: str) -> list[str]:
    sections: list[tuple[str, list[str]]] = []
    for line in text.splitlines():
        marker = ""
        if line.startswith("# HELP "):
            marker = line.split(" ")[2]
        if line == "# EOF":
            marker = "# EOF"
        if marker:
            sections.append((marker, []))
        if sections:
            sections[-1][1].append(line)
    for _, lines in sections:
        lines.sort()
    indexed: list[tuple[str, int]] = [
        (marker, index) for index, (marker, _lines) in enumerate(sections)
    ]
    indexed.sort()
    return ["\n".join(sections[index][1]) for _, index in indexed]


def run(input_value: Input) -> MergeAnswer | ScrapeAnswer:
    with tempfile.TemporaryDirectory() as folder:
        directory = Path(folder)
        paths = write_files(directory, input_value)
        files = [used_hex(path) for path in paths]
        module = prometheus_module()
        registry = CollectorRegistry()
        collector: Collector = module.MultiProcessCollector(registry, str(directory))
        if input_value["call"] == "merge":
            merged = list(collector.merge([str(path) for path in paths], accumulate=True))
            return {"files": files, "families": families_out(merged)}
        scrape = parse_scrape_request(input_value["accept"], "", ())
        body = render_scrape(registry, scrape)
        return {"files": files, "blocks": blocks(body.decode("utf-8"))}
