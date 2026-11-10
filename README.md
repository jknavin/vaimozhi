# VAIMOZHI (வாய்மொழி)

**YOUR VOICE. YOUR WORDS. YOUR WORLD.**

Offline, privacy-first voice typing for Tamil and English, powered by whisper.cpp.

| Phase | Scope | Status |
|---|---|---|
| 1 | Windows offline voice typing (Tauri 2) | **Source complete — build and test on Windows** |
| 2 | Android recording + transcription | Not started |
| 3 | Android VAIMOZHI Keyboard (IME) | Not started |
| 4–6 | Language tuning, optional local-LLM cleanup, polish | Partly in Phase 1 (rule-based cleanup, history, export) |

## What works in Phase 1

- Global shortcut (default `Ctrl+Shift+Space`), toggle or push-to-talk, floating non-focus-stealing indicator, tray, start with Windows
- Real microphone capture (cpal), chunked offline Whisper transcription, silence guard
- Languages: Tamil, English, or Auto (Whisper chooses between Tamil and English only, per stretch of speech)
- Accuracy measures: audio clean-up (DC removal, 80 Hz high-pass, level normalisation), beam search on by default, temperature fallback, script-matched prompts, silence and hallucination filtering, accuracy-first model recommendation (Medium or larger for Tamil)
- Insert into the active app by Ctrl+V with clipboard restore, by simulated typing, or copy-only
- Rule-based cleanup styles (raw, clean, professional, formal, casual, email), filler removal, optional spoken-number conversion
- Model manager: explicit download with progress, resume, SHA-256 check, verify, delete, RAM/disk guidance, recommendation
- SQLite history: search, date filter, favourites, rename, export TXT / DOCX / PDF (print to PDF), editor with undo/redo, retry (needs "keep audio")
- Dark/light themes, English/Tamil UI, no analytics, no accounts, no uploads

## Build (Windows 10/11)

Prerequisites: [Rust](https://rustup.rs) (stable, MSVC toolchain), Visual Studio Build Tools (C++ workload), [Node 20+](https://nodejs.org), [CMake](https://cmake.org), [LLVM](https://releases.llvm.org) (for bindgen; set `LIBCLANG_PATH` to its `bin` folder), WebView2 (preinstalled on Windows 11).

```powershell
git clone <your repo> vaimozhi; cd vaimozhi
cargo test -p vaimozhi-core            # shared logic tests
cd windows
npm install
npx tauri dev                          # run in development
npx tauri build                        # installers in ..\target\release\bundle\{nsis,msi}
```

No Windows machine? Push this repo to GitHub and run **Actions → Build Windows installer**; the `.exe`/`.msi` are uploaded as artifacts.

## First run

1. Open **Models**, pick the recommended model, press **Download** (explicit, with size shown).
2. Settings → Microphone → **Test microphone**.
3. Click into any app, press `Ctrl+Shift+Space`, speak, press it again.

More: [documentation/SETUP.md](documentation/SETUP.md) · [ARCHITECTURE.md](documentation/ARCHITECTURE.md) · [TESTING.md](documentation/TESTING.md) · [TROUBLESHOOTING.md](documentation/TROUBLESHOOTING.md)

## Honest limitations

- **Tamil accuracy depends on the model.** Tiny, Base and Small make many Tamil mistakes; use Medium or larger (Large v3 is the most accurate). Expect errors, especially with names, noise and fast speech.
- Whisper is not streaming: text appears after you stop, and long CPU transcription can take longer than the recording on slow PCs.
- **No speech recogniser is 100% accurate on every voice.** Strong accents, children, fast or mumbled speech, noise and unusual names lower accuracy. For best results use a fixed language (Tamil or English) instead of Auto, a good microphone close to your mouth, and Large v3 (best) or Medium.
- Auto mode decides per stretch of speech, so a sentence that switches between Tamil and English can be written entirely in one of them.
- Cleanup is rule-based only; the optional local-LLM rewrite (Phase 5) is not implemented. Nothing invents or rewrites names or legal wording.
- Paste mode restores text and image clipboards only; files or rich clipboard data are lost. Use "type" or turn restore off if that matters.
- Apps running as Administrator ignore input from a non-elevated VAIMOZHI (Windows limitation).
- Auto-update is not built in: **Check for updates** opens the releases URL you set in `windows/src/config.js`. Signed Tauri updater needs your own signing key.
- The installer is unsigned, so SmartScreen will warn.
- This code was written without access to a Windows build environment; compile errors from crate version drift are possible. Report them and they are quick to fix.
