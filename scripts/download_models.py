#!/usr/bin/env python3
"""
Download and verify models required by gclaw-voice.

Downloads:
  - Silero VAD v5 ONNX model (~2 MB)
  - Whisper.cpp GGML model (base = ~150 MB, tiny = ~75 MB)
  - Piper TTS voice model (optional, ~50 MB)

Usage:
  python scripts/download_models.py                    # Download all (base whisper)
  python scripts/download_models.py --whisper tiny     # Use tiny model
  python scripts/download_models.py --skip-piper       # Skip Piper download
  python scripts/download_models.py --models-dir path  # Custom output directory
"""

import argparse
import hashlib
import os
import sys
import urllib.request

MODELS = {
    "silero_vad": {
        "url": "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx",
        "filename": "silero_vad.onnx",
        "description": "Silero VAD v5 (voice activity detection)",
    },
    "whisper_tiny": {
        "url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        "filename": "ggml-tiny.bin",
        "description": "Whisper tiny (~75 MB, fastest, lowest accuracy)",
    },
    "whisper_base": {
        "url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        "filename": "ggml-base.bin",
        "description": "Whisper base (~150 MB, good balance)",
    },
    "whisper_small": {
        "url": "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        "filename": "ggml-small.bin",
        "description": "Whisper small (~500 MB, good accuracy)",
    },
    "piper_voice": {
        "url": "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx",
        "filename": "en_US-lessac-medium.onnx",
        "description": "Piper TTS English voice (lessac medium, ~50 MB)",
    },
    "piper_voice_config": {
        "url": "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json",
        "filename": "en_US-lessac-medium.onnx.json",
        "description": "Piper TTS voice config",
    },
}


def download_file(url, dest, description):
    """Download a file with progress reporting."""
    if os.path.exists(dest):
        size = os.path.getsize(dest)
        print(f"  [skip] {description} already exists ({size:,} bytes)")
        return True

    print(f"  [download] {description}")
    print(f"    URL: {url}")

    try:
        req = urllib.request.Request(url, headers={"User-Agent": "gclaw-model-downloader/1.0"})
        with urllib.request.urlopen(req) as response:
            total = int(response.headers.get("Content-Length", 0))
            downloaded = 0
            block_size = 8192

            with open(dest + ".tmp", "wb") as f:
                while True:
                    chunk = response.read(block_size)
                    if not chunk:
                        break
                    f.write(chunk)
                    downloaded += len(chunk)
                    if total > 0:
                        pct = downloaded * 100 // total
                        mb = downloaded / (1024 * 1024)
                        total_mb = total / (1024 * 1024)
                        print(f"\r    {mb:.1f}/{total_mb:.1f} MB ({pct}%)", end="", flush=True)

            print()  # Newline after progress.
            os.rename(dest + ".tmp", dest)
            size = os.path.getsize(dest)
            print(f"    Saved: {dest} ({size:,} bytes)")
            return True

    except Exception as e:
        print(f"    ERROR: {e}")
        if os.path.exists(dest + ".tmp"):
            os.remove(dest + ".tmp")
        return False


def main():
    parser = argparse.ArgumentParser(description="Download gclaw-voice models")
    parser.add_argument(
        "--whisper",
        choices=["tiny", "base", "small"],
        default="base",
        help="Whisper model size (default: base)",
    )
    parser.add_argument("--skip-piper", action="store_true", help="Skip Piper TTS download")
    parser.add_argument(
        "--models-dir",
        default=os.path.join(os.path.dirname(os.path.dirname(__file__)), "models"),
        help="Output directory (default: ./models/)",
    )
    args = parser.parse_args()

    models_dir = os.path.abspath(args.models_dir)
    os.makedirs(models_dir, exist_ok=True)

    print(f"G-Claw Model Downloader")
    print(f"Output: {models_dir}\n")

    success = True

    # Silero VAD (always required).
    m = MODELS["silero_vad"]
    if not download_file(m["url"], os.path.join(models_dir, m["filename"]), m["description"]):
        success = False

    # Whisper STT.
    key = f"whisper_{args.whisper}"
    m = MODELS[key]
    if not download_file(m["url"], os.path.join(models_dir, m["filename"]), m["description"]):
        success = False

    # Piper TTS.
    if not args.skip_piper:
        for key in ["piper_voice", "piper_voice_config"]:
            m = MODELS[key]
            if not download_file(m["url"], os.path.join(models_dir, m["filename"]), m["description"]):
                success = False

    print()
    if success:
        print("All models downloaded successfully!")
        print(f"\nUsage:")
        print(f"  gclaw-voice --vad-model {models_dir}/silero_vad.onnx \\")
        print(f"              --whisper-model {models_dir}/ggml-{args.whisper}.bin")
    else:
        print("Some downloads failed. Re-run to retry.")
        sys.exit(1)


if __name__ == "__main__":
    main()
