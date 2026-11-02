import argparse
import re
import shutil
from pathlib import Path

from isolate import active_checkout, git


def cleanup(source: Path, target: Path, expected: str) -> str:
    source, target = source.resolve(), target.resolve()
    if source == target or source in target.parents or not re.fullmatch(r"B[0-9]+", target.name):
        raise RuntimeError("refusing a path outside the task checkout convention")
    if active_checkout(target) is not None:
        raise RuntimeError("checkout still has a live process")
    branch = git(target, "branch", "--show-current").decode().strip()
    if not branch.startswith(f"task/{target.name}-"):
        raise RuntimeError("checkout is on a different task branch")
    if git(target, "rev-parse", "HEAD").decode().strip() != expected or git(target, "status", "--porcelain"):
        raise RuntimeError("checkout changed after its verified merge; preserving it")
    archive = f"refs/betterlite/completed/{target.name}"
    git(source, "fetch", "--no-tags", str(target), f"HEAD:{archive}")
    if git(source, "rev-parse", archive).decode().strip() != expected:
        raise RuntimeError("archived HEAD does not match the verified task")
    if git(target, "status", "--porcelain") or active_checkout(target) is not None:
        raise RuntimeError("checkout changed during archival; preserving it")
    if (target / ".git").is_file():
        git(source, "worktree", "remove", str(target))
    else:
        shutil.rmtree(target)
    return f"archived {expected} at {archive}; removed merged checkout"


class Arguments(argparse.Namespace):
    source: Path
    target: Path
    head: str


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Archive and clean a task after its merged gate passed")
    parser.add_argument("source", type=Path)
    parser.add_argument("target", type=Path)
    parser.add_argument("head")
    args = parser.parse_args(namespace=Arguments())
    print(cleanup(args.source, args.target, args.head))
