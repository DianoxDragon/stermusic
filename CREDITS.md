# Credits

SterMusic is free software, released under the **GNU GPL version 3 or later** (see
[LICENSE](LICENSE)). It builds on other people's work:

## Resources embedded in the program

| Resource | Source | License |
|----------|--------|---------|
| `assets/earth.jpg` (the web radio globe's Earth) | NASA, Blue Marble / GIBS | Public domain |
| `assets/cities.tsv` (the globe's cities) | [GeoNames](https://www.geonames.org), cities15000, filtered | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `assets/icons/` | [Lucide](https://lucide.dev) | ISC ([text](assets/icons/LICENSE)) |
| egui fallback fonts (Ubuntu Light, Hack, Noto Emoji, emoji-icon-font) | through the `epaint_default_fonts` library | Ubuntu Font Licence, MIT, OFL 1.1 |

The globe's detailed tiles (NASA GIBS), the stations (radio-browser.info), lyrics (lrclib.net)
and album art (MusicBrainz / Cover Art Archive) are downloaded while you use the program and are
not part of it.

## Modified libraries (`vendor/` folder)

- `vendor/smithay-clipboard`: Wayland clipboard, modified to accept files dropped onto the window
  (MIT license, see its `LICENSE` and `README.md`).
- `vendor/lofty`: tag reading (lofty 0.22.4), with the fix for "unsynchronised" ID3v2.4 album art
  backported from lofty 0.25 (MIT or Apache-2.0 license, see `STERMUSIC.md`).

## Rust libraries

About 400 libraries (egui, rodio, Symphonia, lofty, image, rustls…), under the MIT, Apache-2.0,
MPL-2.0, ISC, BSD, Zlib and similar licenses, all compatible with the GPL-3.0. The full list,
with the text of each license, ships with the installers (`THIRD-PARTY-LICENSES.txt`, generated
by `packaging/third_party.py`).
