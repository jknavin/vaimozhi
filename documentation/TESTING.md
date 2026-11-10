# Testing

## Automated (run in this project's CI)
`cargo test -p vaimozhi-core` — transliteration, cleanup, numbers, model catalog.
`cargo test -p vaimozhi` — chunking, resampling, WAV, history DB, model hashing, exports.

## Manual checklist (Windows)
| Case | Steps | Expected |
|---|---|---|
| English | Language English, dictate a sentence in Notepad | Capitalised sentence pasted |
| Tamil | Language Tamil, Small+ model, say "வணக்கம் நண்பா" | Tamil script pasted |
| Auto | Language Auto; say one Tamil sentence, then one English sentence | Each written in its own script |
| Accents / other voices | Have 3+ different people dictate the same paragraph in each language | Record word error rate per person and model, and note weak cases |
| Silence / noise only | Record 5 s of room noise | Nothing pasted (no invented phrases) |
| Long recording | Record 3+ min | Progress bar advances, text complete, no crash |
| Background noise | Fan/TV on | Text may degrade; no hallucination on silence |
| Mic denied | Windows → Privacy → Microphone → off | Clear error message in overlay/toast |
| No model | Fresh install, press hotkey | Opens Models with explanation |
| Low storage | Download with little free disk | "Not enough free disk space" before download |
| Clipboard | Copy text, dictate, paste manually | Original text still on clipboard (restore on) |
| Shortcut | Change to `Ctrl+Alt+D`, save | New shortcut works; old one stops; invalid/taken shows error |
| Push-to-talk | Mode push; hold shortcut | Records while held |
| Persistence | Dictate, quit from tray, reopen | History and settings intact |
| Offline | Disconnect network | Dictation works |
| Verify | Corrupt a model by appending a byte; Verify | "does NOT match" |
