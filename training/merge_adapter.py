"""Merge a LoRA adapter into its base model for GGUF conversion.

After merging, convert with llama.cpp's convert_hf_to_gguf.py and quantize,
then add a registry entry in ~/.veyra/models.toml and evaluate it with
`veyra eval` before relying on it.
"""

import argparse
import sys


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--base", required=True)
    ap.add_argument("--adapter", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    try:
        import torch
        from peft import PeftModel
        from transformers import AutoModelForCausalLM, AutoTokenizer
    except ImportError as e:
        print(f"Dependencies missing ({e}). Run: pip install -r requirements.txt")
        return 1
    base = AutoModelForCausalLM.from_pretrained(args.base, torch_dtype=torch.bfloat16)
    merged = PeftModel.from_pretrained(base, args.adapter).merge_and_unload()
    merged.save_pretrained(args.out, safe_serialization=True)
    AutoTokenizer.from_pretrained(args.base).save_pretrained(args.out)
    print(f"Merged model written to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
