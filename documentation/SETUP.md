# Setup

## Install (end user)
Run the `.exe` (per-user, no admin needed) or `.msi`. SmartScreen: "More info → Run anyway" (unsigned build).

## Models
Open **Models** in the app and press Download. Offline/manual alternative:
1. Download a `ggml-*.bin` file listed in the app from https://huggingface.co/ggerganov/whisper.cpp/tree/main on any PC.
2. Copy it into the models folder shown in Settings → Storage (`%APPDATA%\app.vaimozhi.desktop\models`).
3. Press **Verify** in Models once to record its hash.

| Model | Download | RAM | Use |
|---|---|---|---|
| Small (q5_1) | ~181 MB | ~0.7 GB | Weak PCs only; Tamil is fair at best |
| Small | ~466 MB | ~1 GB | Good English, fair Tamil |
| Medium (q5_0) | ~539 MB | ~1.7 GB | Good Tamil and English; best for mid-range PCs |
| Large v3 (q5_0) | ~1.1 GB | ~3.5 GB | Very high accuracy, slow on CPU |
| Large v3 | ~3.1 GB | ~5 GB | Highest accuracy here, needs 8+ GB RAM |
| Large v3 Turbo (q5_0) | ~574 MB | ~1.9 GB | Fast, excellent English; may trail Large v3 on Tamil |

Sizes and RAM are approximate.

## Developer build
See README. Set `LIBCLANG_PATH` (e.g. `C:\Program Files\LLVM\bin`) if bindgen cannot find libclang.
