#!/usr/bin/env python3
"""Line-oriented probe for the pinned upstream GoldStandardAgent.

This helper intentionally imports an already-present checkout.  It has no
network or package-install behavior.

Input records (ASCII, one per line):

    draw|CARD_IDS|TOP_ID|LEGAL_ACTION_IDS
    discard|CARD_IDS|-|LEGAL_ACTION_IDS

The response is `ok|LINE_NUMBER|ACTION_ID` for every input record.
"""

from __future__ import annotations

import argparse
import importlib.metadata
import pathlib
import sys


EXPECTED_PYTHON = (3, 11)
EXPECTED_PACKAGES = {"pettingzoo": "1.24.3", "rlcard": "1.0.5"}


def check_runtime() -> None:
    if sys.version_info[:2] != EXPECTED_PYTHON:
        actual = ".".join(map(str, sys.version_info[:3]))
        raise RuntimeError(f"Python 3.11 is required, found {actual}")
    for package, expected in EXPECTED_PACKAGES.items():
        try:
            actual = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError as error:
            raise RuntimeError(f"{package}=={expected} is required") from error
        if actual != expected:
            raise RuntimeError(
                f"{package}=={expected} is required, found {actual}"
            )
    try:
        import numpy  # noqa: F401
    except ImportError as error:
        raise RuntimeError("NumPy is required by GoldStandardAgent") from error


def parse_ids(field: str) -> list[int]:
    if not field:
        return []
    values = [int(value) for value in field.split(",")]
    if any(value < 0 for value in values):
        raise ValueError("ids must be non-negative")
    return values


def load_agent(root: pathlib.Path):
    source = root / "agents" / "gold_standard_agent.py"
    if not source.is_file():
        raise RuntimeError(f"missing pinned source: {source}")
    sys.path.insert(0, str(root))
    from agents.gold_standard_agent import GoldStandardAgent

    return GoldStandardAgent(env=None)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=pathlib.Path, required=True)
    parser.add_argument(
        "--check", action="store_true", help="only validate the pinned runtime"
    )
    args = parser.parse_args()

    check_runtime()
    agent = load_agent(args.root.resolve())
    if args.check:
        print("ok|runtime|python=3.11|pettingzoo=1.24.3|rlcard=1.0.5")
        return 0

    for line_number, raw in enumerate(sys.stdin, start=1):
        raw = raw.rstrip("\n")
        if not raw:
            continue
        fields = raw.split("|")
        if len(fields) != 4:
            raise ValueError(f"line {line_number}: expected four fields")
        operation, hand_field, top_field, legal_field = fields
        hand = parse_ids(hand_field)
        legal = set(parse_ids(legal_field))
        if operation == "draw":
            top = parse_ids(top_field)
            if len(hand) != 10 or len(top) != 1:
                raise ValueError(
                    f"line {line_number}: draw needs ten hand ids and one top id"
                )
            action = agent._draw_decision(hand, top, legal)
        elif operation == "discard":
            if len(hand) != 11 or top_field != "-":
                raise ValueError(
                    f"line {line_number}: discard needs eleven ids and '-'"
                )
            action = agent._discard_decision(hand, legal)
        else:
            raise ValueError(f"line {line_number}: unknown operation {operation!r}")
        print(f"ok|{line_number}|{int(action)}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # concise subprocess diagnostic for the Rust test
        print(f"error|{type(error).__name__}|{error}", file=sys.stderr)
        raise SystemExit(2) from error
