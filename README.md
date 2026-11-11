# 🎵 SterMusic

*English · [Français](README.fr.md)*

A music player for Linux, written in Rust: the look of the Windows 11 Media Player (sidebar,
album art, large playback bar, Mica), the SterCode design language, and real Linux desktop
integration. The interface is available in English and French (it follows your desktop language,
and can be changed in Settings → Language).

## Features

**Library**
- Several music folders, scanned recursively; a **cache** makes later launches instant, and only
  changed files are read again
- MP3, FLAC, OGG/Vorbis, WAV, AAC/M4A (Symphonia, pure Rust)
- Tags read: title, artist, album artist, album, year, genre, track and disc numbers
- **Album art** embedded in the files, or `cover.jpg` / `folder.jpg` / `front.png`… in the folder
- **Tracks** (sortable by any column), **Albums** (cover grid), **Artists**, **Folders** and
  **Browse** (genre, year, composer) views, album and artist pages
- Instant search (artists, albums, tracks)

**Playback**
- Queue: "Play next", "Add to queue", drag-and-drop reordering, "Save as playlist"
- Reversible shuffle (turning it off restores the original order), "by my taste" ordering,
  repeat (queue / track)
- Gapless playback, crossfade, ReplayGain, bit-perfect output, volume up to 150 %
- **Now Playing** page: large album art on a blurred background, synced lyrics, "Up next",
  true full screen (F11)
- **50 visualizers** rendered on the GPU (Aurora, Vinyl, Hologram, Retro, Eclipse, Stained
  glass, Lissajous…), picked from animated previews, or shuffled with crossfades (automatic mode),
  each with its own settings
- **Mini player**, always on top (Ctrl+M)
- Resume on launch: queue and position restored (paused)
- Sleep timer with fade-out, audio output selection

**Playlists**
- Favorites (the heart), personal playlists with a cover mosaic, smart playlists built from rules,
  up to six playlists pinned in the sidebar
- **M3U** import / export

**Web radio**
- [radio-browser.info](https://www.radio-browser.info) directory: search by name, genre and
  country, "My stations", add a station by its address (.pls, .m3u)
- **Globe**: geolocated stations from all over the world on a 3D Earth (NASA satellite imagery,
  public domain, more detailed as you zoom in), grouped by city; spin and zoom it, click a city
  to list its stations
- Live song title (ICY metadata), automatic reconnection

**Beyond local files**
- **Formats**: besides Symphonia, ffmpeg plays Opus, WMA, APE, WavPack, DSD, Musepack…;
  **CUE sheets** split a single-file album into real tracks
- **Music servers**: Navidrome / Subsonic and Jellyfin, mixed into the library, streamed
- **Podcasts**: search (Apple directory) or RSS feeds, resume where you left off, downloads for
  offline listening
- **Casting** to a Chromecast or a DLNA / UPnP device (TV, speaker, amplifier); what the device
  cannot play is converted on the fly
- **Ratings** from 1 to 5 stars (discreet: shown on hover), stored in the files; Ctrl+1 to
  Ctrl+5 for the current track
- **Statistics**: listening history, complete / partial / skipped plays, yearly recap;
  scrobbling to ListenBrainz and Last.fm

**Tools** (their own page in the sidebar)
- Tag editor (MusicBrainz import, search and replace, rename files from tags), missing album
  art, ReplayGain analysis, duplicates
- **Convert** (MP3, Opus, AAC, Vorbis, FLAC), **rip a CD** (titles from MusicBrainz),
  **sync** playlists to a USB stick, a memory card or a phone (MTP, through KDE)

**Linux desktop**
- **MPRIS**: KDE Plasma / GNOME media widget, media keys, lock screen
- **Breeze style (Kubuntu)**: KDE colors and accent (`kdeglobals`), system Breeze icons, Noto
  Sans font, round window buttons and centered title
- Background of your choice: simulated **Mica** (your KDE Plasma or GNOME wallpaper, blurred,
  tinted, with subtle grain; adjustable strength), **transparent** (blurred by KWin) or solid;
  optional theme that adapts to the current album art
- Tray icon, desktop notifications, drag and drop of files onto the window (Wayland included),
  "Edit tags with SterMusic" in Dolphin's context menu
- **Rounded corners** (square when the window is maximized)
- Fallback fonts for Japanese, Chinese and Korean (Noto Sans CJK)

## Installation

### Ready-made installers

Download them from the [Releases](../../releases) page:

- `stermusic_<version>_amd64.deb` for Kubuntu, Ubuntu, Debian, Linux Mint…: double-click it
  (Discover / Software), or run `sudo apt install ./stermusic_<version>_amd64.deb`;
- `stermusic-<version>-x86_64.tar.gz` for other distributions: extract it, then run
  `./install.sh` (no sudo needed; `./uninstall.sh` removes it).

Requirements: glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Linux Mint 21, Fedora 36 and later)
and ALSA (`libasound2`). **ffmpeg** is recommended (Opus, WMA, APE, WavPack, DSD, conversion,
CD ripping).

To build these installers yourself:

```bash
./packaging/make-installer.sh
```

They are written to `dist/`. Set the package maintainer with `MAINTAINER="Name <address>"` and
the repository address with `HOMEPAGE="https://…"`.

### Build from source

#### 1. Requirements (once)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
sudo apt install libasound2-dev                                 # ALSA headers
```

#### 2. Build and install

```bash
./install.sh
```

The binary goes to `~/.local/bin/stermusic`; SterMusic shows up in the application menu.

To try it without installing:

```bash
cargo run --release -- ~/Music
```

## Keyboard shortcuts

| Key               | Action                          |
|-------------------|---------------------------------|
| Space             | Play / pause                    |
| Ctrl + → / ←      | Next / previous track           |
| → / ←             | Forward / back 5 s              |
| Ctrl + ↑ / ↓      | Volume                          |
| Ctrl + F          | Search                          |
| Ctrl + M          | Mini player                     |
| F11               | Full screen (Now Playing); Esc to leave |
| V / Shift + V     | Next / previous visualizer (Now Playing) |
| Ctrl + 0 … 5      | Rate the current track          |
| Esc               | Back                            |
| Double-click      | Play a track                    |
| Right-click       | Menu (play next, playlists…)    |

## Code layout

- `src/main.rs`: window
- `src/app/mod.rs`: state, queue, actions, saved settings
- `src/app/chrome.rs`: title bar, sidebar, dialogs, notifications
- `src/app/nowbar.rs`: playback bar and mini player
- `src/app/views.rs`: pages (home, library, album, artist, queue, playlists, settings…)
- `src/app/lists.rs`: track lists, grids, context menus
- `src/library.rs`: scanning, tags, cache, albums and artists
- `src/covers.rs`: album art (loaded in the background)
- `src/player.rs`: audio engine (rodio 0.21)
- `src/mpris.rs`: MPRIS integration (souvlaki, pure-Rust D-Bus)
- `src/backdrop.rs`: Mica background
- `src/shader.rs`, `src/viz.rs`, `src/app/viz_view.rs`: visualizers
- `src/radio.rs`, `src/globe.rs`, `src/app/radio_view.rs`, `src/app/radio_globe.rs`: web
  radio, directory and globe
- `src/theme.rs`, `src/anim.rs`, `src/icons.rs`, `src/widgets.rs`: design, animations, icons and
  widgets, shared with SterCode
- `src/i18n.rs`, `src/i18n_en.rs`: English translation (the source strings are in French)

Data: `~/.local/share/stermusic/` (settings, library cache) and `~/.cache/stermusic/` (album art
for MPRIS, downloaded files).

Code comments are written in French.

### Building on NTFS

Cargo build scripts crash on NTFS partitions. If the project lives on one, a local
`.cargo/config.toml` (not tracked by git) can move the build directory elsewhere:

```toml
[build]
target-dir = "/home/me/.cache/stermusic-target"
```

## License

SterMusic is free software: you can redistribute it and/or modify it under the terms of the
**GNU General Public License version 3**, or (at your option) any later version. The full text is
in [LICENSE](LICENSE).

The embedded images, icons and fonts, and the modified libraries in `vendor/`, keep their own
licenses: see [CREDITS.md](CREDITS.md). The installers also include the list of Rust libraries
compiled into the program, with their licenses (`THIRD-PARTY-LICENSES.txt`).
