# What's new in SwiftDM

The newest version is first. Each `## x.y.z` section is the text of that release's page and of the
"What's new" in the app's update window: short lines, in plain words.

## 1.8.3

- **FFmpeg's Update no longer downloads again every time:** SwiftDM remembers which build it installed and says "FFmpeg is up to date" when there is no newer one. (A copy installed by an older SwiftDM is replaced once, to start the memory.)

## 1.8.2

- **Update in Settings → Helper programs now says what happened:** "yt-dlp is up to date (2026.10.21)", or "updated from … to …" and that no restart is needed. It looks for a newer version first, instead of downloading again.

## 1.8.1

- **Kiwi in the dark theme now looks like Network Manager:** near-black surfaces, an olive top bar and lime green accents, flat without the colored glow.

## 1.8.0

- **New color: Kiwi.** A fresh lime green (a deeper green in the light theme), beside Indigo, Blue, Violet, Emerald, Orange and Rose.
- **"What's new" now shows what is new.** The update window lists the changes of the new version, and the page it opens is that version's.
- Fixes found by a new round of code reviews of the 1.7 features:
  - A new link for a download on another site no longer sends the first site's login there.
  - A file that does not match its checksum is deleted (a retry no longer leaves copies), and a checksum is also checked for videos.
  - Download all (browser): the login goes only to the page's own site, the page address is sent as its site only, pictures that link to a larger one are downloaded once, and the button works again after a failure.
  - The program after each download no longer runs the downloaded file by mistake, and finished programs are cleaned up.
  - A text that is not a link stays in the bar; a huge dropped file is refused.
  - Start at sign-in on Linux works with unusual characters in the path.

## 1.7.0

- **Many files with numbers:** `https://site.com/photo[01-30].jpg` becomes thirty links. `[001-120]` keeps its zeros and `[a-f]` goes through letters.
- **Change link…** (the ⋮ menu): an address that expired or moved is replaced, and the download goes on from where it was.
- **Checksums:** give one when you add a file (SHA-256 or SHA-512), or choose **Verify checksum…** later. A file that does not match fails and says so.
- **Browser, "Download all":** the extension lists the pictures, videos, music, documents, archives and programs of a page; one click downloads all of one kind.
- **Browser downloads can start at once**, without the window of choices (Settings → Browser).
- **Run a program for each finished file**, for example a virus scan (Settings → Downloads).
- **Start SwiftDM when you sign in**, minimized (Settings → General).
- **Drop a text, CSV or HTML file** with links on the window to add them all.
- **Browser name (user agent)** for servers that refuse the default; `swiftdm --user-agent` too.

## 1.6.2

- Fixes from five code reviews: pausing never waits for a server that stopped sending; a server that limits requests can no longer leave a file unfinished; speed and time left are right after a resume.
- An unreadable download list is kept as `downloads.json.corrupt` instead of being overwritten.
- Lookups, helper installs, updates and the clipboard watch go on while the window is minimized.
- Browser links are not dropped for an app that does not respond; two starts at the same moment take turns.
- The Linux update package is checked again by the installer; helper programs download over https only.
- The video bar of the extension follows scrolling inside a page, matches links on the real website, and sends no cookies from private windows.
- Arabic: correct counts (one, two, a few, many) for minutes, hours, days and links; menus and paths in the right order.

## 1.6.1

- A server that says "too many requests" is answered with fewer connections instead of retrying all of them.
- Nothing runs off the right edge at the smallest window size; the top bar shows symbols when it is narrow.
- Arabic: the sidebar is on the right, the top bar is mirrored, and icons have room.
- Connection errors are in plain words; menus and dialogs are solid.

## 1.6.0

- **Glass look:** frosted surfaces on a soft colored background (it can be turned off in Settings).
- One text size scale; stronger borders of buttons and fields.

## 1.5.0

- **A progress bar that shows the connections:** each connection fills its own part of the file, with a bright mark where it works now.
- One font on every system (Inter on Linux); calmer file type tiles.

## 1.4.0

- A calmer main screen: the numbers are one slim strip, the type filter ("All types") is next to the search, the row buttons are consistent.
- The Download button is readable while no link is entered; several failures at once are one notice.

## 1.3.4

- Settings, Browser and Help no longer run past the window's edge; text on buttons is readable for every accent color.

## 1.3.3

- **Previews of downloaded pictures** in the list.

## 1.3.2

- A close button on the video bar of the browser extension (it hides on that site); links of Google Images are unwrapped, so the picture is downloaded, not the page around it.

## 1.3.1

- The browser extension is put in your Documents folder (a folder you can find), with a button that copies its path.

## 1.3.0

- More reliable start-up on Windows (a graphics fallback), a normal-sized window on Linux, and a safer hand-over between starts.
- The extension finds the right video on TikTok, Instagram and other feeds.
- Notifications, the schedule and "When done" go on while the window is minimized.

## 1.2.0

- **Browser extension** for Chrome, Edge, Brave, Opera, Vivaldi and Firefox: right-click menus, a button on videos, and downloads the browser starts go to SwiftDM.
- Settings at the top of the window, a Help page, a daily schedule, and "When done" actions (sleep, shut down, quit).

## 1.1.0

- Arabic, German and Turkish.

## 1.0.0

- The first SwiftDM: a fast, free download manager for Windows, macOS and Linux. Several connections per file, resume, videos and music from 1800+ sites, playlists, and a command line.
