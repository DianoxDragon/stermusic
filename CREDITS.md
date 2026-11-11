# Crédits

SterMusic est un logiciel libre, distribué sous la licence **GNU GPL version 3 ou ultérieure**
(fichier [LICENSE](LICENSE)). Il s'appuie sur le travail d'autres personnes :

## Ressources intégrées au programme

| Ressource | Origine | Licence |
|-----------|---------|---------|
| `assets/earth.jpg` (Terre du globe des webradios) | NASA, Blue Marble / GIBS | Domaine public |
| `assets/cities.tsv` (villes du globe) | [GeoNames](https://www.geonames.org), cities15000, filtré | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| `assets/icons/` | [Lucide](https://lucide.dev) | ISC ([texte](assets/icons/LICENSE)) |
| Polices de secours d'egui (Ubuntu Light, Hack, Noto Emoji, emoji-icon-font) | via la bibliothèque `epaint_default_fonts` | Ubuntu Font Licence, MIT, OFL 1.1 |

Les tuiles détaillées du globe (NASA GIBS), les stations (radio-browser.info), les paroles
(lrclib.net) et les pochettes (MusicBrainz / Cover Art Archive) sont téléchargées pendant
l'utilisation et ne font pas partie du programme.

## Bibliothèques modifiées (dossier `vendor/`)

- `vendor/smithay-clipboard` : presse-papiers Wayland, modifié pour le glisser-déposer de
  fichiers (licence MIT, voir son `LICENSE` et son `README.md`).
- `vendor/lofty` : lecture des tags (lofty 0.22.4), avec le correctif des pochettes ID3v2.4
  « désynchronisées » repris de lofty 0.25 (licence MIT ou Apache-2.0, voir `STERMUSIC.md`).

## Bibliothèques Rust

Environ 400 bibliothèques (egui, rodio, Symphonia, lofty, image, rustls…), sous licences
MIT, Apache-2.0, MPL-2.0, ISC, BSD, Zlib et apparentées, toutes compatibles avec la GPL-3.0.
La liste complète, avec le texte de chaque licence, accompagne les installateurs
(`LICENCES-TIERS.txt`, produit par `packaging/third_party.py`).
