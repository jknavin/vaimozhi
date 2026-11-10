# Architecture (Phase 1)

```
vaimozhi/
  shared/core        pure Rust, no dependencies: Language (Tamil/English/Auto), cleanup styles,
                     hallucination filter,
                     spoken numbers, Whisper model catalog + recommendation (25 unit tests)
  windows/src        UI: index.html, style.css, app.js, i18n.js, overlay.html (no framework, no CDN)
  windows/src-tauri  Rust backend
      audio.rs       cpal capture on a dedicated thread → mono → 16 kHz; level meter; pause; WAV I/O
      asr.rs         whisper-rs engine; 28 s chunks cut at the quietest point; silence guard
      modelmgr.rs    download (resume, disk check, SHA-256), verify, delete
      insert.rs      clipboard+Ctrl+V with restore / typing / copy; Win32 focus helpers
      dictation.rs   pipeline + hotkey state machine (toggle / push-to-talk)
      db.rs          SQLite (history + settings key/value)
      export.rs      TXT and DOCX writers
      commands.rs    Tauri commands used by the UI
      main.rs        state, tray, overlay window, shortcut, idle model unload
```

**Pipeline:** hotkey → remember foreground window → record → stop → resample → clean-up filter (DC, 80 Hz high-pass, normalise) → chunk → Whisper (language
code from mode; Auto detects per chunk but only ever picks Tamil or English; a script-matched prompt is used) → hallucination filter
→ cleanup style → hide overlay, refocus the original window → insert → save history.

**Why these choices**
- whisper.cpp via `whisper-rs`: offline, multilingual, CPU-friendly, quantised models.
- Overlay uses `WS_EX_NOACTIVATE` so it never steals focus from the target app.
- Model loads on first use and is freed after N idle minutes (default 10).
- Settings are stored in the local SQLite file. Windows-level encryption of settings was not added: they hold no secrets.
- PDF export uses the webview print dialog ("Save as PDF") because it shapes Tamil correctly; a pure-Rust PDF writer would need a bundled Tamil font and complex-script shaping.

**Privacy:** the only network code is `modelmgr::download`, called only from the Download button. No telemetry.

**Planned (not built):** Android app, IME keyboard, JNI use of `vaimozhi-core`, local-LLM cleanup, live partial results.
