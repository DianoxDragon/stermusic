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
- **Mini lecteur** toujours au premier plan (Ctrl+M)
- Reprise au lancement : file d'attente et position restaurées (en pause)

**Playlists**
- Favoris (le cœur), playlists personnelles avec mosaïque de pochettes
- Import / export **M3U**

**Bureau Linux**
- **MPRIS** : widget lecteur de KDE Plasma / GNOME, touches multimédia, écran de verrouillage
- Fond **Mica** tiré du fond d'écran (KDE Plasma et GNOME)
- Polices du système (Ubuntu, Noto, DejaVu…), dossiers XDG, lanceur `.desktop`
- Seize thèmes de couleurs (ceux de SterCode), barre de titre intégrée

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
- `src/player.rs` : moteur audio (rodio)
- `src/mpris.rs` : intégration MPRIS (souvlaki, D-Bus en pur Rust)
- `src/backdrop.rs` : fond Mica
- `src/theme.rs`, `src/anim.rs`, `src/icons.rs`, `src/widgets.rs` : charte, animations, icônes
  Lucide et widgets, repris de SterCode

Données : `~/.local/share/stermusic/` (réglages, cache de la bibliothèque) et
`~/.cache/stermusic/` (pochettes pour MPRIS).

## Note sur la compilation

Le projet est sur une partition NTFS, où les scripts de compilation de Cargo plantent.
`.cargo/config.toml` envoie donc le dossier de compilation vers `~/.cache/stermusic-target`.
Sur une partition Linux (ext4, btrfs…), ce fichier peut être supprimé.
