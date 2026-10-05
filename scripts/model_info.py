"""Print registry fields for a GGUF file on Hugging Face (see model-info.sh)."""
import json
import sys
import urllib.request

repo, file = sys.argv[1], sys.argv[2]
url = f"https://huggingface.co/api/models/{repo}?blobs=true"
with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "kara-scripts"})) as r:
    d = json.load(r)
s = next((s for s in d["siblings"] if s["rfilename"] == file), None)
if s is None:
    sys.exit(f"{file} not found in {repo}")
lfs = s.get("lfs") or {}
license_ = (d.get("cardData") or {}).get("license", "unknown")
print(f'repo = "{repo}"')
print(f'file = "{file}"')
print(f'revision = "{d["sha"]}"')
print(f'sha256 = "{lfs.get("sha256", "")}"')
print(f"size_bytes = {s.get('size')}")
print(f'license = "{license_}"  # confirm against the model card')
