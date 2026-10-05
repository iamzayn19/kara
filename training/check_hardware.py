"""Decide whether this machine can realistically fine-tune a model.

Prints "Hardware insufficient for this training configuration." and exits 2
when it cannot. Estimates are deliberately conservative.
"""

import argparse
import os
import platform
import re
import shutil
import subprocess
import sys

INSUFFICIENT = "Hardware insufficient for this training configuration."

# Billions of parameters for common names; override with --params-b.
KNOWN = {"0.6b": 0.6, "1.7b": 1.7, "4b": 4.0, "8b": 8.0, "14b": 14.8, "27b": 27.0, "32b": 32.0, "35b": 35.0}


def params_from_name(name: str):
    m = re.search(r"(\d+(?:\.\d+)?)b", name.lower())
    return float(m.group(1)) if m else None


def gpu_memory_gb():
    if shutil.which("nvidia-smi"):
        try:
            out = subprocess.run(
                ["nvidia-smi", "--query-gpu=memory.total", "--format=csv,noheader,nounits"],
                capture_output=True, text=True, timeout=10,
            ).stdout
            vals = [int(x) for x in out.split() if x.strip().isdigit()]
            if vals:
                return max(vals) / 1024, "cuda"
        except (OSError, subprocess.TimeoutExpired):
            pass
    if platform.system() == "Darwin" and platform.machine() == "arm64":
        try:
            total = int(subprocess.run(["sysctl", "-n", "hw.memsize"], capture_output=True, text=True).stdout)
            return total / 2**30 * 0.7, "mps"
        except (OSError, ValueError):
            pass
    return 0.0, "cpu"


def required_gb(params_b: float, method: str) -> float:
    # Rough peak memory: weights + adapters/optimizer + activations at short sequences.
    if method == "qlora":
        return params_b * 0.75 + 4
    if method == "lora":
        return params_b * 2.2 + 6
    return params_b * 16 + 8  # full fine-tune with Adam


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--model", required=True)
    ap.add_argument("--method", choices=["qlora", "lora", "full"], default="qlora")
    ap.add_argument("--params-b", type=float)
    args = ap.parse_args(argv)
    params = args.params_b or params_from_name(args.model)
    if params is None:
        print("Cannot infer model size; pass --params-b.")
        return 1
    have, backend = gpu_memory_gb()
    need = required_gb(params, args.method)
    if args.method == "qlora" and backend != "cuda":
        print(f"{INSUFFICIENT} QLoRA (4-bit) requires an NVIDIA GPU with CUDA; found {backend}.")
        return 2
    if backend == "cpu":
        print(f"{INSUFFICIENT} No supported GPU found; CPU training of a {params}B model is not practical.")
        return 2
    if have < need:
        print(f"{INSUFFICIENT} {args.method} on a {params}B model needs about {need:.0f} GB of accelerator memory; this machine has about {have:.0f} GB ({backend}).")
        return 2
    print(f"OK: {args.method} on a {params}B model needs about {need:.0f} GB; about {have:.0f} GB available ({backend}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
