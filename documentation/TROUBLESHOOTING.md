# Troubleshooting

- **"No microphone found / only silence"** — Windows Settings → Privacy & security → Microphone: allow microphone access and "Let desktop apps access your microphone". Check the input device and volume; use Settings → Test microphone.
- **Shortcut does nothing / "Cannot use …"** — another program owns that combination; choose another. Apps running as Administrator do not receive paste/typing from a normal-privilege VAIMOZHI: run VAIMOZHI as Administrator too.
- **Text not pasted** — try insert method "Type the characters". Some apps (remote desktop, games) block synthetic input.
- **Clipboard content lost** — files/rich data cannot be restored; use "type" or disable restore.
- **Tamil text is wrong** — use Medium or Large v3; set the language to Tamil instead of Auto; speak closer to the mic; reduce noise; keep 'More accurate decoding' on.
- **Slow transcription** — pick a smaller/quantised model, close heavy apps, set CPU threads.
- **"Could not load the model"** — press Verify; delete and download again.
- **Build: libclang not found** — install LLVM and set `LIBCLANG_PATH`. **CMake missing** — install CMake.
- **Build: crate API errors** — versions drifted; check `Cargo.lock` and the version notes at the top of `asr.rs`.
