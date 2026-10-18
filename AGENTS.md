1. Keep it simple: as few settings, files and features as the job needs.
2. Keep the docs minimal: only what you need to run or change the app, in README.md and `docs/`.
3. Never touch the game process, so anti-cheat has nothing to object to: capture the screen with Windows Graphics Capture and game sound with per-process audio capture, never inject code, draw overlays or read game memory, and detect game events only from the captured picture and sound.
4. The repository is public, so write everything in English and never commit secrets, personal data, real recordings or notes, or private hostnames and addresses. Use made-up examples.
5. Everyday features that any review needs live in the app; game-specific analysis lives in plugins.
6. Plugins are self-contained: each one runs as its own program, reads only recordings and set information, writes only events, depends on no other plugin and can be turned on or off on its own. The app knows nothing about any plugin's internals and shows every plugin's events the same way.
7. One mechanism per job: notes, markers and plugin results are all events; the UI shows each action and piece of information in one place, with no explanatory text.
8. Avoid hard-coded numbers; keep the ones you need in one place, with the reason.
9. Make the UI intuitive and standard, like common native apps: standard menus, shortcuts and dialogs; everything works with the mouse alone; modern but simple; a dark, calm look with one accent colour.
10. Keep the code simple: plain functions and modules, no clever abstractions; share code only where it is truly the same; delete what is no longer used; no code comments.
11. Make it robust: nothing is lost by a crash, a dropped connection or someone's delete; everything resumes on its own.
12. Prefer efficiency and simplicity over the lowest delay; a few seconds of lag is fine.
13. Test changes on two PCs, including a fresh install and an update, before releasing.
