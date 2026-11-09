//! Lecture et écriture des tags (ID3v1/v2, MP4, Vorbis, APE…) pour l'éditeur, via lofty.
//!
//! L'éditeur travaille sur une copie des valeurs de chaque fichier ; on n'écrit que les champs
//! qui ont changé. Chaque champ modifié est appliqué à tous les tags présents dans le fichier
//! (par exemple ID3v2 et ID3v1 d'un MP3), pour qu'aucun lecteur ne retrouve l'ancienne valeur.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFile};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    Track,
    TrackTotal,
    Disc,
    DiscTotal,
    Genre,
    Composer,
    Comment,
}

pub const FIELDS: [Field; 12] = [
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Year,
    Field::Track,
    Field::TrackTotal,
    Field::Disc,
    Field::DiscTotal,
    Field::Genre,
    Field::Composer,
    Field::Comment,
];
pub const N: usize = FIELDS.len();

impl Field {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        crate::i18n::tr(match self {
            Field::Title => crate::i18n::n_("Titre"),
            Field::Artist => crate::i18n::n_("Artiste"),
            Field::Album => crate::i18n::n_("Album"),
            Field::AlbumArtist => crate::i18n::n_("Artiste de l'album"),
            Field::Year => crate::i18n::n_("Année"),
            Field::Track => crate::i18n::n_("Piste"),
            Field::TrackTotal => crate::i18n::n_("sur"),
            Field::Disc => crate::i18n::n_("Disque"),
            Field::DiscTotal => crate::i18n::n_("sur"),
            Field::Genre => crate::i18n::n_("Genre"),
            Field::Composer => crate::i18n::n_("Compositeur"),
            Field::Comment => crate::i18n::n_("Commentaire"),
        })
    }

    /// Champ numérique : seuls les chiffres sont acceptés.
    pub fn numeric(self) -> bool {
        matches!(
            self,
            Field::Year | Field::Track | Field::TrackTotal | Field::Disc | Field::DiscTotal
        )
    }
}

pub type Values = [String; N];

/// Pochette intégrée : octets de l'image, et une empreinte pour comparer vite.
#[derive(Clone, PartialEq)]
pub struct Cover {
    pub data: std::sync::Arc<Vec<u8>>,
    pub hash: u64,
}

impl Cover {
    pub fn new(data: Vec<u8>) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut h);
        Self {
            hash: h.finish(),
            data: std::sync::Arc::new(data),
        }
    }
}

pub struct FileTags {
    pub path: PathBuf,
    pub values: Values,
    pub cover: Option<Cover>,
    /// Format et tags présents, pour information (« MP3 · ID3v2, ID3v1 »).
    pub kind: String,
    /// Tags présents seulement (« ID3v2, ID3v1 »), pour la colonne « Tag ».
    pub tags: String,
    pub duration_ms: u64,
    pub size: u64,
    /// Le fichier n'a pas pu être lu : on ne l'écrira pas.
    pub error: Option<String>,
}

fn tag_name(t: TagType) -> &'static str {
    match t {
        TagType::Ape => "APEv2",
        TagType::Id3v1 => "ID3v1",
        TagType::Id3v2 => "ID3v2",
        TagType::Mp4Ilst => "MP4",
        TagType::VorbisComments => "Vorbis",
        TagType::RiffInfo => "RIFF",
        TagType::AiffText => "AIFF",
        _ => "?",
    }
}

fn get(tag: &Tag, field: Field) -> String {
    let s = |v: Option<std::borrow::Cow<str>>| v.map(|s| s.trim().to_owned()).unwrap_or_default();
    let n = |v: Option<u32>| v.map(|n| n.to_string()).unwrap_or_default();
    match field {
        Field::Title => s(tag.title()),
        Field::Artist => s(tag.artist()),
        Field::Album => s(tag.album()),
        Field::AlbumArtist => tag
            .get_string(&ItemKey::AlbumArtist)
            .unwrap_or("")
            .trim()
            .to_owned(),
        Field::Year => n(tag.year()),
        Field::Track => n(tag.track()),
        Field::TrackTotal => n(tag.track_total()),
        Field::Disc => n(tag.disk()),
        Field::DiscTotal => n(tag.disk_total()),
        Field::Genre => s(tag.genre()),
        Field::Composer => tag.get_string(&ItemKey::Composer).unwrap_or("").trim().to_owned(),
        Field::Comment => s(tag.comment()),
    }
}

fn set(tag: &mut Tag, field: Field, value: &str) {
    let value = value.trim();
    let num = value.parse::<u32>().ok().filter(|&n| n > 0);
    let text = (!value.is_empty()).then(|| value.to_owned());
    match field {
        Field::Title => match text {
            Some(v) => tag.set_title(v),
            None => tag.remove_title(),
        },
        Field::Artist => match text {
            Some(v) => tag.set_artist(v),
            None => tag.remove_artist(),
        },
        Field::Album => match text {
            Some(v) => tag.set_album(v),
            None => tag.remove_album(),
        },
        Field::Genre => match text {
            Some(v) => tag.set_genre(v),
            None => tag.remove_genre(),
        },
        Field::Comment => match text {
            Some(v) => tag.set_comment(v),
            None => tag.remove_comment(),
        },
        Field::AlbumArtist | Field::Composer => {
            let key = if field == Field::AlbumArtist {
                ItemKey::AlbumArtist
            } else {
                ItemKey::Composer
            };
            match text {
                Some(v) => {
                    tag.insert_text(key, v);
                }
                None => tag.remove_key(&key),
            }
        }
        Field::Year => match num {
            Some(n) => tag.set_year(n),
            None => tag.remove_year(),
        },
        Field::Track => match num {
            Some(n) => tag.set_track(n),
            None => tag.remove_track(),
        },
        Field::TrackTotal => match num {
            Some(n) => tag.set_track_total(n),
            None => tag.remove_track_total(),
        },
        Field::Disc => match num {
            Some(n) => tag.set_disk(n),
            None => tag.remove_disk(),
        },
        Field::DiscTotal => match num {
            Some(n) => tag.set_disk_total(n),
            None => tag.remove_disk_total(),
        },
    }
}

fn front_cover(tagged: &TaggedFile) -> Option<Cover> {
    let pictures: Vec<&Picture> = tagged.tags().iter().flat_map(|t| t.pictures().iter()).collect();
    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    Some(Cover::new(pic.data().to_vec()))
}

/// Lit les tags d'un fichier (tag principal, sinon le premier trouvé).
pub fn read(path: &Path) -> FileTags {
    let mut out = FileTags {
        path: path.to_path_buf(),
        values: Default::default(),
        cover: None,
        kind: String::new(),
        tags: String::new(),
        duration_ms: 0,
        size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        error: None,
    };
    match Probe::open(path).and_then(|p| p.read()) {
        Ok(tagged) => {
            let format = path
                .extension()
                .map(|e| e.to_string_lossy().to_uppercase())
                .unwrap_or_default();
            let tags: Vec<&str> = tagged.tags().iter().map(|t| tag_name(t.tag_type())).collect();
            out.tags = if tags.is_empty() {
                "—".to_owned()
            } else {
                tags.join(", ")
            };
            out.duration_ms = tagged.properties().duration().as_millis() as u64;
            out.kind = if tags.is_empty() {
                crate::trf!("{format} · sans tags", format)
            } else {
                format!("{format} · {}", tags.join(", "))
            };
            if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                for f in FIELDS {
                    out.values[f.index()] = get(tag, f);
                }
            }
            out.cover = front_cover(&tagged);
        }
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}

/// Nouvelle pochette : `None` la retire, `Some` la remplace.
pub type CoverChange = Option<Cover>;

#[derive(Clone, Copy)]
pub struct WriteOpts {
    /// ID3v2.3 plutôt que 2.4 (MP3), pour les vieux lecteurs et l'Explorateur de Windows.
    pub id3v23: bool,
    /// Garde la date de modification (sinon l'album remonte dans « Ajoutés récemment »).
    pub keep_mtime: bool,
    /// Écrit dans une copie qui remplace l'original : pour le morceau en cours de lecture,
    /// que le décodeur lit encore (une écriture sur place décalerait le son sous ses pieds).
    pub via_copy: bool,
}

fn apply(tag: &mut Tag, fields: &[(Field, String)], cover: Option<&CoverChange>) {
    for (f, v) in fields {
        set(tag, *f, v);
    }
    // Pochettes déjà présentes sans description : même souci qu'en bas en ID3v2.3.
    for i in 0..tag.pictures().len() {
        let p = &tag.pictures()[i];
        if p.description().is_none_or(str::is_empty) {
            let fixed = Picture::new_unchecked(
                p.pic_type(),
                p.mime_type().cloned(),
                Some("Cover".to_owned()),
                p.data().to_vec(),
            );
            tag.set_picture(i, fixed);
        }
    }
    if let Some(change) = cover {
        tag.remove_picture_type(PictureType::CoverFront);
        // MP4 ne connaît pas les types d'image : toutes ses images sont des pochettes.
        if tag.tag_type() == TagType::Mp4Ilst {
            while !tag.pictures().is_empty() {
                tag.remove_picture(0);
            }
        }
        if let Some(c) = change {
            let mime = match c.data.get(..4) {
                Some([0x89, b'P', b'N', b'G']) => Some(MimeType::Png),
                Some([0xFF, 0xD8, ..]) => Some(MimeType::Jpeg),
                _ => None,
            };
            // Une description vide s'écrit en UTF-16 sans marque d'ordre des octets en ID3v2.3,
            // et le fichier ne se relit plus : on en donne une.
            tag.push_picture(Picture::new_unchecked(
                PictureType::CoverFront,
                mime,
                Some("Cover".to_owned()),
                c.data.as_ref().clone(),
            ));
        }
    }
}

/// Écrit les champs modifiés (et la pochette si `cover` est donné) dans le fichier.
pub fn write(
    path: &Path,
    fields: &[(Field, String)],
    cover: Option<&CoverChange>,
    opts: WriteOpts,
) -> Result<(), String> {
    modify(path, opts, |tag| {
        // ID3v1 ne connaît pas les pochettes.
        let cover = if tag.tag_type() == TagType::Id3v1 {
            None
        } else {
            cover
        };
        apply(tag, fields, cover);
    })
}

/// Volume mesuré d'un morceau et de son album (ReplayGain 2.0) : gains en dB, crêtes de 0 à 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayGain {
    pub track_gain: f32,
    pub track_peak: f32,
    pub album_gain: f32,
    pub album_peak: f32,
}

const REPLAYGAIN_KEYS: [ItemKey; 4] = [
    ItemKey::ReplayGainTrackGain,
    ItemKey::ReplayGainTrackPeak,
    ItemKey::ReplayGainAlbumGain,
    ItemKey::ReplayGainAlbumPeak,
];

/// Inscrit les valeurs ReplayGain dans les tags (le son du fichier n'est pas touché).
pub fn write_replaygain(path: &Path, rg: &ReplayGain, opts: WriteOpts) -> Result<(), String> {
    let values = [
        format!("{:.2} dB", rg.track_gain),
        format!("{:.6}", rg.track_peak),
        format!("{:.2} dB", rg.album_gain),
        format!("{:.6}", rg.album_peak),
    ];
    modify(path, opts, |tag| {
        // Les tags qui ne connaissent pas ReplayGain (ID3v1, RIFF) refusent simplement la valeur.
        for (key, value) in REPLAYGAIN_KEYS.iter().zip(&values) {
            tag.insert_text(key.clone(), value.clone());
        }
    })
}

/// Valeurs ReplayGain déjà inscrites dans le fichier ; `None` s'il en manque une.
pub fn read_replaygain(path: &Path) -> Option<ReplayGain> {
    let tagged = Probe::open(path).and_then(|p| p.read()).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let number = |key: &ItemKey| -> Option<f32> {
        let text = tag.get_string(key)?.trim().to_ascii_lowercase();
        text.trim_end_matches("db").trim().replace(',', ".").parse().ok()
    };
    Some(ReplayGain {
        track_gain: number(&REPLAYGAIN_KEYS[0])?,
        track_peak: number(&REPLAYGAIN_KEYS[1])?,
        album_gain: number(&REPLAYGAIN_KEYS[2])?,
        album_peak: number(&REPLAYGAIN_KEYS[3])?,
    })
}

/// Ouvre le fichier, laisse `change` modifier chacun de ses tags, puis le réécrit.
fn modify(path: &Path, opts: WriteOpts, change: impl Fn(&mut Tag)) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mtime: Option<SystemTime> = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut tagged = Probe::open(path).and_then(|p| p.read()).map_err(|e| err(&e))?;

    // Le tag principal est créé s'il manque ; les autres tags présents reçoivent aussi les changements.
    let primary = tagged.primary_tag_type();
    if tagged.tag(primary).is_none() {
        tagged.insert_tag(Tag::new(primary));
    }
    let types: Vec<TagType> = tagged.tags().iter().map(Tag::tag_type).collect();
    for t in types {
        if let Some(mut tag) = tagged.remove(t) {
            change(&mut tag);
            tagged.insert_tag(tag);
        }
    }

    let mut options = WriteOptions::default();
    let options = options.use_id3v23(opts.id3v23);
    let target = if opts.via_copy {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let tmp = path.with_file_name(format!(".{name}.stermusic-tmp"));
        std::fs::copy(path, &tmp).map_err(|e| err(&e))?;
        tmp
    } else {
        path.to_path_buf()
    };
    let mut result = tagged.save_to_path(&target, options).map_err(|e| err(&e));
    // Filet de sécurité : lofty écrit parfois en ID3v2.3 une trame qu'il ne sait plus relire
    // (texte UTF-16 vide). Le fichier est alors réécrit en ID3v2.4, toujours lisible.
    if opts.id3v23 && result.is_ok() && Probe::open(&target).and_then(|p| p.read()).is_err() {
        let mut v24 = WriteOptions::default();
        result = tagged
            .save_to_path(&target, v24.use_id3v23(false))
            .map_err(|e| err(&e));
    }
    if let (true, Some(t)) = (opts.keep_mtime && result.is_ok(), mtime) {
        if let Ok(f) = std::fs::File::options().write(true).open(&target) {
            let _ = f.set_modified(t);
        }
    }
    if opts.via_copy {
        if result.is_ok() {
            std::fs::rename(&target, path).map_err(|e| err(&e))?;
        } else {
            let _ = std::fs::remove_file(&target);
        }
    }
    result
}

/// Fichiers audio d'une liste de chemins (les dossiers sont parcourus), triés. Le parcours
/// s'arrête dès que `stop` passe à vrai (import annulé).
pub fn expand_until(paths: impl IntoIterator<Item = PathBuf>, stop: &AtomicBool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&p)
                .follow_links(true)
                .into_iter()
                .take_while(|_| !stop.load(Ordering::Relaxed))
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file() && crate::library::is_audio_file(e.path()))
                .map(|e| e.into_path())
                .collect();
            files.sort();
            out.extend(files);
        } else if crate::library::is_audio_file(&p) {
            out.push(p);
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// Titre déduit du nom de fichier : « 03 - Artiste - Titre.mp3 » → « Titre ».
pub fn title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem.replace('_', " ");
    // Numéro de piste en tête (« 03 », « 03. », « 1-03 », « 03 - »).
    let rest = stem.trim_start_matches(|c: char| c.is_ascii_digit());
    // Un nom fait seulement de chiffres (« 1979 ») est un titre, pas un numéro.
    let rest = if rest.len() < stem.len() && !rest.is_empty() {
        rest.trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '-' | '.' | ' ' | ')'))
    } else {
        &stem
    };
    // « Artiste - Titre » : on garde la dernière partie.
    let title = rest.rsplit(" - ").next().unwrap_or(rest);
    let title = title.trim();
    if title.is_empty() {
        stem.trim().to_owned()
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aller-retour sur de vrais fichiers : `STERMUSIC_TAG_SAMPLES=a.mp3:b.flac cargo test -- --ignored`
    /// (les fichiers sont copiés, jamais modifiés).
    #[test]
    #[ignore]
    fn write_round_trip() {
        let samples = std::env::var("STERMUSIC_TAG_SAMPLES").unwrap_or_default();
        let dir = std::env::temp_dir().join("stermusic-tag-test");
        std::fs::create_dir_all(&dir).unwrap();
        let png = {
            let img = image::RgbImage::from_pixel(64, 64, image::Rgb([200, 40, 90]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            Cover::new(out.into_inner())
        };
        for src in samples.split(':').filter(|s| !s.is_empty()) {
            let src = Path::new(src);
            for via_copy in [false, true] {
                let dst = dir.join(format!(
                    "{}-{}",
                    via_copy,
                    src.file_name().unwrap().to_string_lossy()
                ));
                std::fs::copy(src, &dst).unwrap();
                let before = std::fs::metadata(&dst).unwrap().modified().unwrap();
                let fields = vec![
                    (Field::Title, "Titre « éè » 漢字".to_owned()),
                    (Field::AlbumArtist, "Artiste d'album".to_owned()),
                    (Field::Year, "1999".to_owned()),
                    (Field::Track, "7".to_owned()),
                    (Field::TrackTotal, "12".to_owned()),
                    (Field::Composer, "Compo".to_owned()),
                    (Field::Comment, String::new()),
                ];
                let opts = WriteOpts {
                    id3v23: via_copy,
                    keep_mtime: true,
                    via_copy,
                };
                write(&dst, &fields, Some(&Some(png.clone())), opts).unwrap();
                let back = read(&dst);
                assert!(back.error.is_none(), "{:?}", back.error);
                for (f, v) in &fields {
                    assert_eq!(&back.values[f.index()], v, "{} {:?}", dst.display(), f);
                }
                assert_eq!(
                    back.cover.as_ref().map(|c| c.hash),
                    Some(png.hash),
                    "{}",
                    dst.display()
                );
                assert_eq!(std::fs::metadata(&dst).unwrap().modified().unwrap(), before);
                // La piste se lit encore jusqu'au bout, et sa durée n'a pas bougé.
                let t0 = crate::library::read_track(src);
                let t1 = crate::library::read_track(&dst);
                assert!((t0.duration_ms as i64 - t1.duration_ms as i64).abs() < 50);
                write(&dst, &[], Some(&None), opts).unwrap();
                assert!(read(&dst).cover.is_none());
                // ReplayGain : écrit, relu, et les autres tags n'ont pas bougé.
                let rg = ReplayGain {
                    track_gain: -7.25,
                    track_peak: 0.987654,
                    album_gain: 1.5,
                    album_peak: 1.0,
                };
                assert!(read_replaygain(&dst).is_none());
                write_replaygain(&dst, &rg, opts).unwrap();
                assert_eq!(read_replaygain(&dst), Some(rg), "{}", dst.display());
                assert_eq!(crate::dsp::replaygain(&dst), Some(-7.25));
                assert_eq!(read(&dst).values[Field::Title.index()], fields[0].1);
                println!("ok {} ({})", dst.display(), back.kind);
            }
        }
    }

    #[test]
    fn titles_from_filenames() {
        let t = |s: &str| title_from_filename(Path::new(s));
        assert_eq!(t("03 - Artiste - Mon titre.mp3"), "Mon titre");
        assert_eq!(t("03. Mon titre.flac"), "Mon titre");
        assert_eq!(t("1-03 Mon_titre.flac"), "Mon titre");
        assert_eq!(t("Mon titre.ogg"), "Mon titre");
        assert_eq!(t("1979.mp3"), "1979");
    }
}
