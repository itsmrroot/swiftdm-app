---
name: swiftdm-windows-debug
description: Debug and fix SwiftDM on a Windows machine (x64 or ARM64). Use when asked to test, debug or fix SwiftDM on Windows, e.g. the in-app update, the browser extension / "New download" window, installer problems, or yt-dlp login errors. Works from PowerShell and logs, makes its own git branch, and never touches the user's real install or settings.
---

# SwiftDM on Windows: debug and fix

You are on a Windows PC with the SwiftDM source. Reproduce problems with commands, find the cause in the code,
fix it on a branch, and prove the fix. Work from PowerShell, files and logs. No screenshots, no clicking.
If you did not see it in output, write "NOT VERIFIED". Never guess.

## 0. Rules

- Work on a branch, never on `main`:
  `git switch main; git pull --ff-only; git switch -c win-debug/<short-topic>-<yyyymmdd>`
  Commit in small steps on that branch. Do NOT push, tag or release unless the user says so.
  Never touch `RELEASES_TOKEN`, tags, or the SwiftDM-release repo.
- Never use the user's real data. Always set a scratch data folder before running the app:
  `$W="$env:TEMP\swiftdm-qa"; New-Item -ItemType Directory $W -Force; $env:SWIFTDM_DATA_DIR="$W\data"`
- Check for a real install first. If one exists, do NOT run the installer or the update test against it:
  `Get-ItemProperty HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\* | ? DisplayName -like "*SwiftDM*" | select DisplayName,DisplayVersion,InstallLocation`
  Test installers only into `$W\app` with `/S /D=$W\app` (`/D=` last, never quoted).
- Do not sign in anywhere, do not read browser profiles or cookies, do not delete files outside `$W`.
- Do not edit registry keys that exist already. Read with `reg query` only.
- No new dependencies unless a fix needs one and you explain why.

## 1. Know the code (read before you edit)

Rust workspace: core crate `swiftdm` (`src/`: http.rs, manager.rs, media.rs, tools.rs, util.rs, paths.rs, main.rs CLI)
and GUI crate `swiftdm-gui` (`gui/src/`: app.rs, add.rs, helpers.rs, update.rs, browser.rs, instance.rs,
settings.rs, theme.rs, i18n.rs, translations.rs, logfile.rs). Style: match the surrounding code, comments only
where the code is not obvious. Texts shown to users go through `i18n.rs`/`translations.rs` (ar/de/tr tables;
a test checks every key is used and translated).

Where things live:

| Area | Files |
| --- | --- |
| In-app update (check, download, install, relaunch) | `gui/src/update.rs`, `gui/src/app.rs` (`on_exit`, update toast) |
| Browser extension, native messaging, registry | `gui/src/browser.rs`, `gui/src/instance.rs`, `gui/assets/browser-extension/*` |
| "New download" window and whether it is skipped | `gui/src/app.rs` `add_requests` (`quick = ...`), `gui/src/add.rs` |
| yt-dlp args, file names, cookies from browser | `src/media.rs`, `src/manager.rs` `run_media`, `gui/src/add.rs` `look` |
| Installer (NSIS via cargo-packager) | `Cargo.toml`/`gui/Cargo.toml` packager metadata, `.github/workflows/release.yml` |

Data folder `%APPDATA%\SwiftDM` (or `$env:SWIFTDM_DATA_DIR`): `swiftdm.log`, `swiftdm.old.log`, `helper.log`
(messages from the extension), `instance-port`, `update\` (downloaded setup), `update-pending.txt`
(written when an update is handed to its setup; the next start shows "was not installed" if still older),
`browser-seen`, `tools\`.

## 2. Build and check (Windows)

```powershell
cargo fmt --check
$env:RUSTFLAGS="-D warnings"; cargo clippy --workspace --all-targets; Remove-Item Env:RUSTFLAGS
cargo test --workspace
cargo build -p swiftdm -p swiftdm-gui          # debug build, target\debug\swiftdm-gui.exe
```

Debug-only env vars: `SWIFTDM_SIZE=WxH`, `SWIFTDM_PAGE`, `SWIFTDM_LANG`. If MSVC build tools or Rust are
missing, say so and stop that part; do not install toolchains without asking.

## 3. Reproduce with evidence

Pick the reported problem and reproduce it first. Save evidence (command output, log lines) in `$W\evidence`.

### 3a. Update does not install / old version after update
The app runs the new setup as `<setup>.exe /S [/R] /D=<exe folder>` (detached) after closing itself.
Reproduce without the GUI, against a scratch folder:
```powershell
$REL="https://github.com/itsmrroot/SwiftDM-release/releases/download"
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {"arm64"} else {"x64"}
iwr "$REL/v1.10.3/swiftdm-windows-$arch-setup.exe" -OutFile $W\old.exe
iwr "$REL/v1.10.5/swiftdm-windows-$arch-setup.exe" -OutFile $W\new.exe
$d="$W\app"; (Start-Process $W\old.exe -ArgumentList "/S","/D=$d" -Wait -PassThru).ExitCode
$app=Start-Process "$d\swiftdm-gui.exe" -PassThru; Start-Sleep 8        # old app running
$r=Start-Process $W\new.exe -ArgumentList "/S","/R","/D=$d" -PassThru; $r.WaitForExit(); $r.ExitCode
Start-Sleep 8; (Get-Item "$d\swiftdm-gui.exe").VersionInfo.FileVersion
Get-Process swiftdm-gui -EA 0 | select Id,Path,StartTime; Get-Content "$W\data\swiftdm.log" -Tail 20
```
Expect exit 0, new version, old process gone, exactly one new process (because of `/R`).
Repeat 3 times; also try: folder with a space (`/D=$W\app dir`, unquoted), old app not running, a file locked
for 20 s (expect a clean failure, not a silent success), run without `/R` (must not start the app).
For the code path itself, read `gui/src/update.rs` `install` and the `swiftdm.log` lines "update: ...".

### 3b. Extension starts downloads without the choices window, or does nothing
1. Run the helper protocol by hand (4-byte little-endian length + JSON on stdin/stdout):
   `swiftdm-gui.exe chrome-extension://fjeibcdeopjapibjafigloehhleijepj/` and send `{"type":"ping"}`.
   Expect `{"ok":true,"version":"..."}` and a `message ping` line in `helper.log`.
2. With the GUI running, send a download message with `"ask":true`, `"ask":false` and no `ask`; read
   `swiftdm.log` for `link from the browser: ... ask=... quick=...`. `quick` must be false for `ask:true`
   unless Settings → Browser "Start them at once, without the window of choices" is on, or the same link
   was downloaded before (forces the dialog).
3. Registry (read only): `reg query "HKCU\Software\<Microsoft\Edge | Google\Chrome | BraveSoftware\Brave-Browser>\NativeMessagingHosts\io.github.itsmrroot.swiftdm"`
   and check the default value points to an existing manifest json whose `path` exists and whose
   `allowed_origins` contains `chrome-extension://fjeibcdeopjapibjafigloehhleijepj/`.
4. `node --check` on the extension scripts; `manifest.json` "version" must equal the app version.

### 3c. "login of the chosen browser could not be read" (TikTok/YouTube)
Chrome/Edge cookie files are locked or app-bound encrypted on new Windows builds. Run
`swiftdm.exe <url> ...` with and without a browser login; check the code falls back to no login once
(`tried_plain` in `manager.rs`, `look()` in `add.rs`) and the message says what to do. Do not read or copy
any cookie file.

### 3d. Installer
Silent install to `$W\app`, check exit code, `DisplayVersion`, shortcuts, and that the license page shows plain
text (no `#`, `<https...>`): unpack with `7z x` if available and inspect. Uninstall with `$d\uninstall.exe /S`.

## 4. Fix

1. State the root cause in one or two sentences with the evidence (file:line, log line). If you cannot find a
   proven cause, add logging that will prove it next time rather than changing behaviour on a guess.
2. Make the smallest change that fixes it. Add or adjust a test when the logic is testable without a GUI.
3. Re-run section 2 (fmt, clippy with `-D warnings`, test) and the reproduction from section 3: it must now pass.
4. Commit on your branch with a message that says what and why, ending with the attribution line the harness gives you.
   One commit per fix. Do NOT bump versions, edit CHANGELOG release sections, tag or push; the user releases
   from the Mac.

## 5. Report

Write `$env:USERPROFILE\Desktop\swiftdm-win-report.md` and print it:

- Machine: Windows version, CPU architecture, antivirus (`Get-MpComputerStatus` if allowed), default browser.
- Branch name and `git log --oneline main..HEAD`.
- For each problem: reproduced yes/no with evidence, root cause, the fix (commit hash), proof it works
  (command output), and anything NOT VERIFIED.
- Anything you saw that is wrong but did not fix.
- The exact commands for the user to publish the branch when they decide: `git push -u origin <branch>`.

Stop after the report. Do not merge, release, or open issues or pull requests.
