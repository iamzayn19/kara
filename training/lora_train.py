"""(Q)LoRA fine-tuning of a small model on Veyra-format examples.

Experimental and optional. Requires `pip install -r requirements.txt` and a
supported GPU (check_hardware.py). Converts each example into a chat
transcript: system prompt, user task, assistant tool calls, tool results.
"""

import argparse
import glob
import json
import os
import sys

from check_hardware import INSUFFICIENT, main as check_hardware

SYSTEM = "You are Veyra, a local software engineering agent. Act through tools; verify changes with tests."


def to_messages(ex: dict) -> list[dict]:
    msgs = [{"role": "system", "content": SYSTEM}, {"role": "user", "content": ex["task"]}]
    for i, step in enumerate(ex["steps"]):
        call_id = f"call_{i}"
        msgs.append({
            "role": "assistant",
            "content": "",
            "tool_calls": [{"id": call_id, "type": "function", "function": {"name": step["tool"], "arguments": step["arguments"]}}],
        })
        msgs.append({"role": "tool", "tool_call_id": call_id, "content": step.get("summary") or ("ok" if step["ok"] else "error")})
    msgs.append({"role": "assistant", "content": "Done. The relevant tests pass."})
    return msgs


def load_examples(data_dir: str) -> list[dict]:
    rows = []
    for f in sorted(glob.glob(os.path.join(data_dir, "*.jsonl"))):
        with open(f, encoding="utf-8") as fh:
            for line in fh:
                ex = json.loads(line)
                if ex.get("consent", {}).get("explicit") is not True:
                    continue
                rows.append({"messages": to_messages(ex)})
    return rows


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--model", required=True, help="Hugging Face model id, e.g. Qwen/Qwen3-1.7B")
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--method", choices=["qlora", "lora"], default="qlora")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--rank", type=int, default=16)
    args = ap.parse_args(argv)

    if check_hardware(["--model", args.model, "--method", args.method]) != 0:
        return 2
    rows = load_examples(args.data)
    if len(rows) < 20:
        print(f"Only {len(rows)} examples; collect more before training (at least 20, ideally hundreds).")
        return 1

    try:
        import torch
        from datasets import Dataset
        from peft import LoraConfig, get_peft_model, prepare_model_for_kbit_training
        from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig, Trainer, TrainingArguments
    except ImportError as e:
        print(f"Training dependencies missing ({e}). Run: pip install -r requirements.txt")
        return 1

    tok = AutoTokenizer.from_pretrained(args.model)
    quant = BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_compute_dtype=torch.bfloat16) if args.method == "qlora" else None
    model = AutoModelForCausalLM.from_pretrained(args.model, quantization_config=quant, torch_dtype=torch.bfloat16)
    if quant:
        model = prepare_model_for_kbit_training(model)
    model = get_peft_model(model, LoraConfig(r=args.rank, lora_alpha=args.rank * 2, lora_dropout=0.05, task_type="CAUSAL_LM",
                                             target_modules=["q_proj", "k_proj", "v_proj", "o_proj"]))

    def encode(row):
        text = tok.apply_chat_template(row["messages"], tokenize=False)
        ids = tok(text, truncation=True, max_length=4096)
        ids["labels"] = ids["input_ids"].copy()
        return ids

    ds = Dataset.from_list(rows).map(encode, remove_columns=["messages"])
    trainer = Trainer(
        model=model,
        train_dataset=ds,
        args=TrainingArguments(output_dir=args.out, num_train_epochs=args.epochs, per_device_train_batch_size=1,
                               gradient_accumulation_steps=8, learning_rate=2e-4, logging_steps=10, save_strategy="epoch",
                               bf16=True, report_to=[]),
    )
    trainer.train()
    model.save_pretrained(args.out)
    tok.save_pretrained(args.out)
    print(f"Adapter saved to {args.out}. Merge with merge_adapter.py, convert to GGUF with llama.cpp, then evaluate with `veyra eval` before using it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
