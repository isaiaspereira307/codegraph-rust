#!/usr/bin/env python3
# ABOUTME: Generates a deterministic synthetic Python repository for scale measurements.
# ABOUTME: Usage: gen_synthetic.py <out-dir> --modules N [--seed S]; ~20 graph nodes per module.
"""Synthetic Python repo for `docs/parity/scale.md`.

Each module holds one class with METHODS methods and FUNCTIONS top-level functions (plus its
File node: about 20 nodes per module). Modules live in packages of PACKAGE_SIZE. Every function
calls up to two functions of its own module and one imported from another module (`from pkg.mod
import f`) of the same package or, one time in ten, of an earlier package (layered packages); methods call a sibling method through `self` and one module function, and every tenth
module has a test module whose tests call into it. The output depends only on the arguments.
"""

import argparse
import random
from pathlib import Path

METHODS = 8
FUNCTIONS = 10
PACKAGE_SIZE = 50


def module_name(i: int) -> tuple[str, str]:
    return f"pkg{i // PACKAGE_SIZE:03d}", f"mod{i % PACKAGE_SIZE:03d}"


def func(i: int, j: int) -> str:
    return f"f{i}_{j}"


def callee_module(i: int, n: int, rng: random.Random) -> int:
    """Another module of the same package, or (1 in 10) any module of an earlier package."""
    first = (i // PACKAGE_SIZE) * PACKAGE_SIZE
    if first > 0 and rng.random() < 0.1:
        return rng.randrange(first)
    size = min(PACKAGE_SIZE, n - first)
    if size == 1:
        return (i + 1) % n
    other = first + rng.randrange(size - 1)
    return other + 1 if other >= i else other


def module_source(i: int, n: int, rng: random.Random) -> str:
    imports: dict[str, set[str]] = {}
    calls: list[tuple[int, int, str]] = []
    for j in range(FUNCTIONS):
        other = callee_module(i, n, rng)
        k = rng.randrange(FUNCTIONS)
        package, mod = module_name(other)
        imports.setdefault(f"{package}.{mod}", set()).add(func(other, k))
        calls.append((rng.randrange(FUNCTIONS), rng.randrange(FUNCTIONS), func(other, k)))
    lines = [f'"""Synthetic module {i}."""', ""]
    for target in sorted(imports):
        names = ", ".join(sorted(imports[target]))
        lines.append(f"from {target} import {names}")
    lines += ["", ""]
    for j in range(FUNCTIONS):
        a, b, external = calls[j]
        body = []
        if a != j:
            body.append(f"    x = {func(i, a)}(value)")
        else:
            body.append("    x = value")
        if b != j and b != a:
            body.append(f"    x += {func(i, b)}(x)")
        body.append(f"    return {external}(x) + {j}")
        lines.append(f"def {func(i, j)}(value):")
        lines += body
        lines += ["", ""]
    lines.append(f"class Service{i}:")
    lines.append(f'    """Service of module {i}."""')
    lines.append("")
    for m in range(METHODS):
        lines.append(f"    def method{m}(self, value):")
        if m + 1 < METHODS:
            lines.append(f"        value = self.method{m + 1}(value)")
        lines.append(f"        return {func(i, m % FUNCTIONS)}(value)")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def test_source(i: int) -> str:
    package, mod = module_name(i)
    lines = [
        f'"""Tests for module {i}."""',
        "",
        f"from {package}.{mod} import Service{i}, {func(i, 0)}, {func(i, 1)}",
        "",
        "",
        f"def test_{func(i, 0)}():",
        f"    assert {func(i, 0)}(1) is not None",
        "",
        "",
        f"def test_{func(i, 1)}():",
        f"    assert {func(i, 1)}(2) is not None",
        "",
        "",
        f"def test_service{i}():",
        f"    assert Service{i}().method0(3) is not None",
    ]
    return "\n".join(lines) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument("--modules", type=int, required=True)
    parser.add_argument("--seed", type=int, default=14)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    n = args.modules
    out: Path = args.out
    out.mkdir(parents=True, exist_ok=False)
    for p in range((n + PACKAGE_SIZE - 1) // PACKAGE_SIZE):
        pkg = out / f"pkg{p:03d}"
        pkg.mkdir()
        (pkg / "__init__.py").write_text("")
    (out / "tests").mkdir()
    for i in range(n):
        package, mod = module_name(i)
        (out / package / f"{mod}.py").write_text(module_source(i, n, rng))
        if i % 10 == 0:
            (out / "tests" / f"test_{package}_{mod}.py").write_text(test_source(i))


if __name__ == "__main__":
    main()
