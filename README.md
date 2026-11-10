# 🎵 SterMusic

Un lecteur de musique pour Linux, écrit en Rust : l'allure du lecteur multimédia de Windows 11
(barre latérale, pochettes, grande barre de lecture, Mica), la charte de SterCode, et une
vraie intégration au bureau Linux.

## Fonctionnalités

**Bibliothèque**
- Plusieurs dossiers de musique, parcourus récursivement ; **cache** : les lancements suivants
  sont instantanés, et seuls les fichiers modifiés sont relus
- MP3, FLAC, OGG/Vorbis, WAV, AAC/M4A (Symphonia, en pur Rust)
- Tags lus : titre, artiste, artiste de l'album, album, année, genre, n° de piste et de disque
- **Pochettes** intégrées aux fichiers, ou `cover.jpg` / `folder.jpg` / `front.png`… du dossier
- Vues **Morceaux** (tri par titre, artiste, album ou date d'ajout), **Albums** (grille de
  pochettes) et **Artistes**, pages d'album et d'artiste
- Recherche instantanée (artistes, albums, morceaux)

**Lecture**
- File d'attente : « Lire ensuite », « Ajouter à la file », réorganisation par glisser-déposer,
  « Enregistrer comme playlist »
- Aléatoire réversible (couper l'aléatoire rend l'ordre d'origine) et répétition (file / morceau)
- Page **En cours de lecture** : grande pochette sur fond flouté, « À suivre »
- **38 visualiseurs** dessinés par la carte graphique (Aurore, Vinyle, Hologramme, Ferrofluide,
  Topographie, Lissajous…), choisis d'après leurs aperçus animés, ou tirés au hasard en fondu
  enchaîné (mode automatique)
- **Mini lecteur** toujours au premier plan (Ctrl+M)
- Reprise au lancement : file d'attente et position restaurées (en pause)

**Playlists**
- Favoris (le cœur), playlists personnelles avec mosaïque de pochettes
- Import / export **M3U**

**Webradios**
- Annuaire [radio-browser.info](https://www.radio-browser.info) : recherche par nom, genre et
  pays, « Mes stations », ajout d'une station par son adresse (.pls, .m3u)
- **Globe** : les stations géolocalisées du monde entier sur une Terre en 3D (image satellite
  de la NASA, domaine public, plus détaillée quand on zoome), rangées par ville ; on la fait
  tourner et zoomer, un clic sur une ville liste ses stations
- Titre diffusé en direct (métadonnées ICY), reconnexion automatique

**Au-delà des fichiers**
- **Formats** : en plus de Symphonia, ffmpeg lit l'Opus, le WMA, l'APE, le WavPack, le DSD, le
  Musepack… ; les **feuilles CUE** découpent un album en un seul fichier en vrais morceaux
- **Serveurs de musique** : Navidrome / Subsonic et Jellyfin, mélangés à la bibliothèque, lus
  en flux
- **Podcasts** : recherche (annuaire d'Apple) ou flux RSS, reprise là où on s'était arrêté,
  téléchargement pour l'écoute hors ligne
- **Diffusion** vers un Chromecast ou un appareil DLNA / UPnP (TV, enceinte, ampli) ; ce que
  l'appareil ne lit pas est converti à la volée
- **Notes** de 1 à 5 étoiles (discrètes : au survol), Ctrl+1 à Ctrl+5 pour le morceau en cours

**Outils** (page à part dans la barre latérale)
- Éditeur de tags, pochettes manquantes, ReplayGain, doublons
- **Convertir** (MP3, Opus, AAC, Vorbis, FLAC), **extraire un CD** (titres trouvés sur
  MusicBrainz), **synchroniser** des playlists vers une clé, une carte ou un téléphone (MTP,
  via KDE)

**Bureau Linux**
- **MPRIS** : widget lecteur de KDE Plasma / GNOME, touches multimédia, écran de verrouillage
- **Style Breeze (Kubuntu)**, par défaut sous KDE : couleurs et accent de KDE (`kdeglobals`),
  icônes Breeze du système, police Noto Sans, boutons de fenêtre ronds et titre centré
- Fond au choix : **Mica** simulé (fond d'écran de KDE Plasma ou GNOME, flouté, teinté, avec un
  grain discret ; intensité réglable), **transparent** (flouté par KWin : bouton pour ajouter
  SterMusic à l'effet Better Blur) ou uni
- **Coins arrondis** (droits quand la fenêtre est agrandie)
- Polices de secours pour le japonais, le chinois et le coréen (Noto Sans CJK)
- Dossiers XDG, lanceur `.desktop`, seize thèmes de SterCode en plus de celui de KDE

## Installation

### 1. Prérequis (une seule fois)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
sudo apt install libasound2-dev                                 # en-têtes ALSA
```

### 2. Compiler et installer

```bash
./install.sh
```

Le binaire va dans `~/.local/bin/stermusic` ; SterMusic apparaît dans le menu des applications.

Pour essayer sans installer :

```bash
cargo run --release -- ~/Musique
```

## Raccourcis clavier

| Touche            | Action                          |
|-------------------|---------------------------------|
| Espace            | Lecture / pause                 |
| Ctrl + → / ←      | Morceau suivant / précédent     |
| → / ←             | Avancer / reculer de 5 s        |
| Ctrl + ↑ / ↓      | Volume                          |
| Ctrl + F          | Rechercher                      |
| Ctrl + M          | Mini lecteur                    |
| V / Maj + V       | Visualiseur suivant / précédent (En cours de lecture) |
| Échap             | Retour                          |
| Double-clic       | Lancer un morceau               |
| Clic droit        | Menu (lire ensuite, playlists…) |

## Structure du code

- `src/main.rs` : fenêtre
- `src/app/mod.rs` : état, file d'attente, actions, réglages enregistrés
- `src/app/chrome.rs` : barre de titre, barre latérale, dialogues, notifications
- `src/app/nowbar.rs` : barre de lecture et mini lecteur
- `src/app/views.rs` : pages (accueil, bibliothèque, album, artiste, file, playlists, réglages…)
- `src/app/lists.rs` : listes de morceaux, grilles, menus contextuels
- `src/library.rs` : scan, tags, cache, albums et artistes
- `src/covers.rs` : pochettes (chargées en arrière-plan)
- `src/player.rs` : moteur audio (rodio 0.21 ; M4A/AAC compris)
- `src/mpris.rs` : intégration MPRIS (souvlaki, D-Bus en pur Rust)
- `src/backdrop.rs` : fond Mica
- `src/radio.rs`, `src/globe.rs`, `src/app/radio_view.rs`, `src/app/radio_globe.rs` :
  webradios, annuaire et globe (`assets/earth.jpg` : Blue Marble avec relief, NASA GIBS,
  domaine public ; détails téléchargés au zoom depuis GIBS ; `assets/cities.tsv` : villes de
  plus de 15 000 habitants, [GeoNames](https://www.geonames.org), CC BY 4.0)
- `src/theme.rs`, `src/anim.rs`, `src/icons.rs`, `src/widgets.rs` : charte, animations, icônes
  Lucide et widgets, repris de SterCode

Données : `~/.local/share/stermusic/` (réglages, cache de la bibliothèque) et
`~/.cache/stermusic/` (pochettes pour MPRIS).

## Note sur la compilation

Le projet est sur une partition NTFS, où les scripts de compilation de Cargo plantent.
`.cargo/config.toml` envoie donc le dossier de compilation vers `~/.cache/stermusic-target`.
Sur une partition Linux (ext4, btrfs…), ce fichier peut être supprimé.
