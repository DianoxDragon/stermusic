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
    save_safely(path, opts, |target| {
        let mut result = tagged.save_to_path(target, options).map_err(|e| err(&e));
        // Filet de sécurité : lofty écrit parfois en ID3v2.3 une trame qu'il ne sait plus relire
        // (texte UTF-16 vide). Le fichier est alors réécrit en ID3v2.4, toujours lisible.
        if opts.id3v23 && result.is_ok() && Probe::open(target).and_then(|p| p.read()).is_err() {
            let mut v24 = WriteOptions::default();
            result = tagged
                .save_to_path(target, v24.use_id3v23(false))
                .map_err(|e| err(&e));
        }
        result
    })
}

fn is_mp3(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"))
}

/// Réécrit un MP3 (voir [`save_safely`]).
fn save_mp3(path: &Path, mp3: &lofty::mpeg::MpegFile, opts: WriteOpts) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    save_safely(path, opts, |target| {
        let mut options = WriteOptions::default();
        let mut result = mp3
            .save_to_path(target, options.use_id3v23(opts.id3v23))
            .map_err(|e| err(&e));
        // Même filet de sécurité qu'en général : ID3v2.4 si l'ID3v2.3 ne se relit pas.
        if opts.id3v23 && result.is_ok() && Probe::open(target).and_then(|p| p.read()).is_err() {
            let mut v24 = WriteOptions::default();
            result = mp3
                .save_to_path(target, v24.use_id3v23(false))
                .map_err(|e| err(&e));
        }
        result
    })
}

/// Écrit le fichier par `save` : dans une copie qui remplace l'original si `via_copy`, en
/// gardant sa date de modification si `keep_mtime`.
fn save_safely(
    path: &Path,
    opts: WriteOpts,
    save: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mtime: Option<SystemTime> = std::fs::metadata(path).and_then(|m| m.modified()).ok();
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
    let result = save(&target);
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

// ---------- Notes (étoiles) ----------
//
// Les notes sont écrites dans les fichiers, comme le font les autres lecteurs : elles suivent
// le morceau d'un logiciel à l'autre et survivent à une réinstallation.
// - MP3 (ID3v2) : trame POPM, sur l'échelle de Windows et de MusicBee (1, 64, 128, 196, 255) ;
//   les trames POPM d'autres logiciels sont mises à jour, une est ajoutée s'il n'y en a pas.
// - FLAC, Ogg, Opus, APE, MP4 : FMPS_RATING (0,2 à 1,0, spécification freedesktop « FMPS »,
//   comme Strawberry, Clementine ou Amarok). Un RATING déjà présent (foobar2000, MusicBee) est
//   mis à jour dans son échelle (1 à 5, ou 0 à 100) ; en MP4, l'atome « rate » (0 à 100) aussi.

/// Note d'une trame POPM (0 à 255) en étoiles.
fn popm_stars(byte: u8) -> Option<u8> {
    match byte {
        0 => None,
        1..=31 => Some(1),
        32..=95 => Some(2),
        96..=159 => Some(3),
        160..=223 => Some(4),
        _ => Some(5),
    }
}

/// Étoiles en note POPM (échelle de Windows Media Player, reprise par MusicBee).
fn popm_byte(stars: u8) -> u8 {
    [0, 1, 64, 128, 196, 255][stars.min(5) as usize]
}

/// Texte d'une note (FMPS 0–1, ou RATING 1–5 / 0–100) en étoiles.
fn text_stars(text: &str, fmps: bool) -> Option<u8> {
    let v: f32 = text.trim().replace(',', ".").parse().ok()?;
    let stars = if fmps {
        v * 5.0
    } else if v <= 5.0 {
        v
    } else {
        v / 20.0
    };
    let stars = stars.round();
    (stars >= 1.0).then(|| stars.min(5.0) as u8)
}

fn is_fmps(key: &ItemKey) -> bool {
    match key {
        ItemKey::Unknown(k) => {
            let k = k.rsplit(':').next().unwrap_or(k);
            k.eq_ignore_ascii_case("FMPS_RATING")
        }
        _ => false,
    }
}

/// Note lue dans un tag (sauf POPM, que seul l'ID3v2 concret connaît).
pub fn tag_rating(tag: &Tag) -> Option<u8> {
    let fmps = tag
        .items()
        .find(|i| is_fmps(i.key()))
        .and_then(|i| i.value().text())
        .and_then(|t| text_stars(t, true));
    fmps.or_else(|| {
        tag.get_string(&ItemKey::Popularimeter)
            .and_then(|t| text_stars(t, false))
    })
}

/// Note des trames POPM d'un tag ID3v2 (la première qui en a une).
pub fn id3v2_rating(tag: &lofty::id3::v2::Id3v2Tag) -> Option<u8> {
    tag.into_iter().find_map(|f| match f {
        lofty::id3::v2::Frame::Popularimeter(p) => popm_stars(p.rating),
        _ => None,
    })
}

/// Note du fichier (`None` : aucune).
#[cfg(test)]
pub fn read_rating(path: &Path) -> Option<u8> {
    if is_mp3(path) {
        let mut f = std::fs::File::open(path).ok()?;
        let mp3 = lofty::mpeg::MpegFile::read_from(
            &mut f,
            lofty::config::ParseOptions::new().read_properties(false),
        )
        .ok()?;
        return mp3.id3v2().and_then(id3v2_rating);
    }
    let tagged = Probe::open(path).and_then(|p| p.read()).ok()?;
    tagged.tags().iter().find_map(tag_rating)
}

/// Inscrit la note `stars` (1 à 5 ; 0 : la retire) dans le fichier.
pub fn write_rating(path: &Path, stars: u8, opts: WriteOpts) -> Result<(), String> {
    let stars = stars.min(5);
    if is_mp3(path) {
        return write_mp3_rating(path, stars, opts);
    }
    modify(path, opts, |tag| set_text_rating(tag, stars))
}

/// Note des tags texte (Vorbis, APE, MP4).
fn set_text_rating(tag: &mut Tag, stars: u8) {
    let tag_type = tag.tag_type();
    if !matches!(tag_type, TagType::VorbisComments | TagType::Ape | TagType::Mp4Ilst) {
        return;
    }
    // RATING (ou « rate » en MP4) déjà là : mis à jour dans l'échelle qu'il utilise.
    let old_scale_5 = tag
        .get_string(&ItemKey::Popularimeter)
        .and_then(|t| t.trim().replace(',', ".").parse::<f32>().ok())
        .map(|v| v <= 5.0 && tag_type != TagType::Mp4Ilst);
    let fmps_key = match tag.items().find(|i| is_fmps(i.key())) {
        Some(i) => i.key().clone(),
        None if tag_type == TagType::Mp4Ilst => ItemKey::Unknown("----:com.apple.iTunes:FMPS_Rating".into()),
        None => ItemKey::Unknown("FMPS_RATING".into()),
    };
    tag.retain(|i| !is_fmps(i.key()));
    tag.remove_key(&ItemKey::Popularimeter);
    if stars == 0 {
        return;
    }
    tag.insert_unchecked(lofty::tag::TagItem::new(
        fmps_key,
        lofty::tag::ItemValue::Text(format!("{:.1}", stars as f32 / 5.0)),
    ));
    let rating = match old_scale_5 {
        Some(true) => Some(stars.to_string()),
        Some(false) => Some((stars as u32 * 20).to_string()),
        None if tag_type == TagType::Mp4Ilst => Some((stars as u32 * 20).to_string()),
        None => None,
    };
    if let Some(r) = rating {
        tag.insert_text(ItemKey::Popularimeter, r);
    }
}

fn write_mp3_rating(path: &Path, stars: u8, opts: WriteOpts) -> Result<(), String> {
    use lofty::id3::v2::{Frame, PopularimeterFrame};
    let err = |e: &dyn std::fmt::Display| e.to_string();
    let mut mp3 = {
        let mut f = std::fs::File::open(path).map_err(|e| err(&e))?;
        lofty::mpeg::MpegFile::read_from(&mut f, lofty::config::ParseOptions::new()).map_err(|e| err(&e))?
    };
    let mut tag = mp3.remove_id3v2().unwrap_or_default();
    let mut popm: Vec<PopularimeterFrame<'static>> = (&tag)
        .into_iter()
        .filter_map(|f| match f {
            Frame::Popularimeter(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    tag.retain(|f| !matches!(f, Frame::Popularimeter(_)));
    if popm.is_empty() && stars > 0 {
        popm.push(PopularimeterFrame::new("Windows Media Player 9 Series".into(), 0, 0));
    }
    for mut p in popm {
        p.rating = popm_byte(stars);
        // Sans note ni compteur d'écoutes, la trame n'a plus de raison d'être.
        if p.rating > 0 || p.counter > 0 {
            tag.insert(Frame::Popularimeter(p));
        }
    }
    mp3.set_id3v2(tag);
    save_mp3(path, &mp3, opts)
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
                assert_eq!(crate::dsp::replaygain(&dst, false), Some(-7.25));
                assert_eq!(read(&dst).values[Field::Title.index()], fields[0].1);
                println!("ok {} ({})", dst.display(), back.kind);
            }
        }
    }

    /// Notes écrites puis relues dans chaque format, et gardées quand l'éditeur de tags
    /// réécrit le fichier (fichiers d'essai fabriqués par ffmpeg ; sauté sans lui).
    #[test]
    fn ratings_round_trip() {
        if !crate::ffdecode::available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("stermusic-rating-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["mp3", "flac", "ogg", "opus", "m4a"] {
            let path = dir.join(format!("essai.{ext}"));
            let ok = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
                .arg(&path)
                .status()
                .is_ok_and(|s| s.success());
            assert!(ok, "ffmpeg {ext}");
            let opts = WriteOpts { id3v23: ext == "mp3", keep_mtime: true, via_copy: false };
            assert_eq!(read_rating(&path), None, "{ext}");
            write_rating(&path, 4, opts).unwrap();
            assert_eq!(read_rating(&path), Some(4), "{ext}");
            // Une autre trame inconnue de lofty (MP3) doit survivre, elle aussi.
            if ext == "mp3" {
                let mut f = std::fs::File::open(&path).unwrap();
                let mut mp3 = lofty::mpeg::MpegFile::read_from(&mut f, lofty::config::ParseOptions::new()).unwrap();
                let mut tag = mp3.remove_id3v2().unwrap();
                tag.insert(lofty::id3::v2::Frame::Private(lofty::id3::v2::PrivateFrame::new(
                    "stermusic-test".into(),
                    vec![1, 2, 3],
                )));
                mp3.set_id3v2(tag);
                save_mp3(&path, &mp3, opts).unwrap();
            }
            // L'éditeur de tags réécrit le fichier : la note reste.
            write(&path, &[(Field::Title, "Titre".into())], None, opts).unwrap();
            assert_eq!(read_rating(&path), Some(4), "{ext} après l'éditeur");
            assert_eq!(read(&path).values[Field::Title.index()], "Titre", "{ext}");
            if ext == "mp3" {
                let mut f = std::fs::File::open(&path).unwrap();
                let mp3 = lofty::mpeg::MpegFile::read_from(&mut f, lofty::config::ParseOptions::new()).unwrap();
                let private = mp3.id3v2().unwrap().into_iter().any(|f| matches!(f, lofty::id3::v2::Frame::Private(_)));
                assert!(private, "trame PRIV perdue");
            }
            write_rating(&path, 2, WriteOpts { via_copy: true, ..opts }).unwrap();
            assert_eq!(read_rating(&path), Some(2), "{ext} par copie");
            write_rating(&path, 0, opts).unwrap();
            assert_eq!(read_rating(&path), None, "{ext} retirée");
            let t = crate::library::read_track(&path);
            assert!(t.duration_ms > 900, "{ext} : {} ms", t.duration_ms);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rating_scales() {
        assert_eq!(popm_stars(popm_byte(3)), Some(3));
        for n in 1..=5 {
            assert_eq!(popm_stars(popm_byte(n)), Some(n));
        }
        assert_eq!(popm_stars(0), None);
        assert_eq!(text_stars("0.6", true), Some(3));
        assert_eq!(text_stars("3", false), Some(3));
        assert_eq!(text_stars("80", false), Some(4));
        assert_eq!(text_stars("0", false), None);
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
