//! Status portraits for the twelve crew captains.
//!
//! The embedded set is illustrated and uses one still per captain. A different set can be
//! dropped in without a rebuild: point `SHINA_SPRITES` at a folder containing
//! `<captain>/Normal.png`, or add `<workspace>/assets/crew` with the same layout. Missing
//! expressions use that captain's Normal face. An absent captain uses the embedded artwork.
//!
//! Only pixel sets with all four valid animation strips (`<Clip>.strip.png`) play them in
//! the sidebar. The original animated set is available in `assets/crew-pixel`.
use rune::{Image, ImageCache, Pixels, Waker};
use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

/// How many portraits ship. Saved captains store an index, so new portraits only append.
pub const PORTRAITS: usize = 12;
/// Display names, in the same order as the captain indices used by threads.
/// Portrait names in the captain editor describe the art, not a captain identity.
pub const PORTRAIT_NAMES: [&str; PORTRAITS] = [
    "Copper", "Mint", "Indigo", "Gold", "Navy", "Mustard", "Hood", "Braids", "Mohawk", "Silver",
    "Chrome", "Russet",
];
pub const CAPTAIN_NAMES: [&str; PORTRAITS] = [
    "Ember", "Juniper", "Sable", "Pip", "Bram", "Wren", "Rook", "Nia", "Jett", "Otto", "Bolt",
    "Kit",
];
/// Folder names used by the sprite-set loader.
pub const CAPTAIN_KEYS: [&str; PORTRAITS] = [
    "ember", "juniper", "sable", "pip", "bram", "wren", "rook", "nia", "jett", "otto", "bolt",
    "kit",
];
/// The work each captain is responsible for.
pub const CAPTAIN_ROLES: [&str; PORTRAITS] = [
    "Platform",
    "Release",
    "Support",
    "Research",
    "Operations",
    "Interface",
    "Security",
    "Data",
    "Performance",
    "Review",
    "Automation",
    "Testing",
];
pub const CAPTAIN_ROLE_DETAIL: [&str; PORTRAITS] = [
    "Architecture and implementation",
    "Release planning and validation",
    "Support investigations and fixes",
    "Long-horizon research and refactors",
    "Deploys, incidents and on-call follow-up",
    "Interface polish, layout and accessibility",
    "Security reviews and hardening",
    "Schemas, migrations and data pipelines",
    "Profiling, latency and memory use",
    "Code review and mentoring",
    "CI, build scripts and tooling",
    "Test coverage and flaky failures",
];
const FACE_FILES: [&str; Face::COUNT] =
    ["Normal", "Happy", "Determined", "Surprised", "Dizzy", "Sad"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Normal,
    Happy,
    Determined,
    Surprised,
    Dizzy,
    Sad,
}

impl Face {
    pub const COUNT: usize = Self::Sad.index() + 1;
    pub const fn index(self) -> usize {
        match self {
            Self::Normal => 0,
            Self::Happy => 1,
            Self::Determined => 2,
            Self::Surprised => 3,
            Self::Dizzy => 4,
            Self::Sad => 5,
        }
    }
}

/// A short animation for one status. Frame 0 of every strip is the matching static face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Clip {
    /// Ready: breathing and a blink, built on Normal.
    Idle,
    /// Working: typing dots and a nod, built on Determined.
    Working,
    /// Needs you: a startled hop, built on Surprised.
    Attention,
    /// Done: one small hop, built on Happy.
    Happy,
}

impl Clip {
    pub const ALL: [Clip; 4] = [Clip::Idle, Clip::Working, Clip::Attention, Clip::Happy];

    /// The clip a status plays, if any. Failed stays on its static face.
    pub fn for_face(face: Face) -> Option<Clip> {
        match face {
            Face::Normal => Some(Clip::Idle),
            Face::Determined => Some(Clip::Working),
            Face::Surprised => Some(Clip::Attention),
            Face::Happy => Some(Clip::Happy),
            Face::Dizzy | Face::Sad => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    fn file(self) -> &'static str {
        match self {
            Clip::Idle => "Idle",
            Clip::Working => "Working",
            Clip::Attention => "Attention",
            Clip::Happy => "Happy",
        }
    }

    /// Frames in the strip, as `tools/crew_originals.py` draws them.
    pub fn frames(self) -> usize {
        match self {
            Clip::Happy => 3,
            _ => 4,
        }
    }

    /// (frame, milliseconds) steps. Idle changes about once a second, Working about four times
    /// a second, Attention hops then holds for five seconds, and Happy plays once.
    pub fn timeline(self) -> &'static [(usize, u32)] {
        match self {
            Clip::Idle => &[
                (0, 1700),
                (1, 1400),
                (0, 1600),
                (1, 1400),
                (0, 600),
                (2, 50),
                (3, 90),
                (2, 50),
            ],
            Clip::Working => &[
                (1, 220),
                (2, 220),
                (0, 260),
                (1, 220),
                (2, 220),
                (0, 260),
                (1, 220),
                (2, 220),
                (0, 260),
                (3, 110),
            ],
            Clip::Attention => &[
                (1, 110),
                (2, 90),
                (0, 120),
                (1, 110),
                (2, 90),
                (3, 700),
                (0, 5200),
            ],
            Clip::Happy => &[(1, 120), (2, 100), (0, 120), (1, 120), (2, 100)],
        }
    }

    /// Whether the clip loops. A one-shot clip ends on frame 0 and stays there.
    pub fn repeats(self) -> bool {
        self != Clip::Happy
    }
}

/// One clip's frames as RGBA, ready for `rune::Pixels::set`.
pub struct Strip {
    pub size: u32,
    /// How many pixels one art cell spans in the frames, from `crisp::upscale`.
    pub cell: u32,
    pub frames: Vec<Vec<u8>>,
}

impl Strip {
    /// Splits a strip of square frames. None unless it has exactly the clip's frame count.
    fn decode(png: &[u8], clip: Clip) -> Option<Self> {
        let sheet = image::load_from_memory(png).ok()?.to_rgba8();
        let size = sheet.height();
        if size == 0 || sheet.width() != size * clip.frames() as u32 {
            return None;
        }
        let frames: Vec<Vec<u8>> = (0..clip.frames() as u32)
            .map(|i| {
                // Copy contiguous RGBA rows instead of visiting every pixel through
                // GenericImageView, which is costly in unoptimized test builds.
                let row_bytes = size as usize * 4;
                let start = i as usize * row_bytes;
                let mut frame = Vec::with_capacity(row_bytes * size as usize);
                for row in sheet.as_raw().chunks_exact(sheet.width() as usize * 4) {
                    frame.extend_from_slice(&row[start..start + row_bytes]);
                }
                frame
            })
            .collect();
        let cell = crate::crisp::upscale(size, size, &frames[0]);
        Some(Self { size, cell, frames })
    }

    /// Frame `index` as a `target` pixel square of whole scaled cells.
    pub fn scaled(&self, index: usize, target: u32) -> Vec<u8> {
        crate::crisp::fit(&self.frames[index], self.size, self.size, self.cell, target)
    }
}

/// Every clip for one captain, or none.
pub type Motion = [Strip; 4];

/// One face as the PNG it ships in, decoded the first time a size asks for it.
#[derive(Clone)]
struct Art {
    png: Arc<[u8]>,
    decoded: Arc<OnceLock<Option<Decoded>>>,
}

struct Decoded {
    width: u32,
    height: u32,
    cell: u32,
    rgba: Vec<u8>,
}

impl Art {
    fn new(png: impl Into<Arc<[u8]>>) -> Self {
        Self {
            png: png.into(),
            decoded: Arc::new(OnceLock::new()),
        }
    }

    fn decoded(&self) -> Option<&Decoded> {
        self.decoded
            .get_or_init(|| {
                let image = image::load_from_memory(&self.png).ok()?.to_rgba8();
                let (width, height) = image.dimensions();
                let rgba = image.into_raw();
                let cell = crate::crisp::upscale(width, height, &rgba);
                Some(Decoded {
                    width,
                    height,
                    cell,
                    rgba,
                })
            })
            .as_ref()
    }
}

type Set = [Art; Face::COUNT];

/// What one scaled buffer is of: a face (captain, face index, head crop or bust) or a mark.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Shown {
    Face(usize, usize, bool),
    /// The upper bust: a `BUST_WINDOW` square of cells cut from the full bust.
    Window(usize, usize),
    Mark(bool),
    Mascot(ProviderMascot, bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderMascot {
    OpenAi,
    Anthropic,
    Google,
    DeepSeek,
    Glm,
    Qwen,
    Grok,
    Default,
}

impl ProviderMascot {
    const ALL: [Self; 8] = [
        Self::OpenAi,
        Self::Anthropic,
        Self::Google,
        Self::DeepSeek,
        Self::Glm,
        Self::Qwen,
        Self::Grok,
        Self::Default,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::DeepSeek => "deepseek",
            Self::Glm => "glm",
            Self::Qwen => "qwen",
            Self::Grok => "grok",
            Self::Default => "default",
        }
    }
}

pub fn provider_mascot_for(model_or_provider: &str) -> ProviderMascot {
    use ProviderMascot::*;
    let name = model_or_provider.to_ascii_lowercase();
    for (mascot, names) in [
        (Anthropic, &["anthropic", "claude"][..]),
        (Google, &["google", "gemini", "antigravity"]),
        (DeepSeek, &["deepseek"]),
        (Glm, &["glm", "zhipu", "z.ai"]),
        (Qwen, &["qwen", "alibaba"]),
        (Grok, &["grok", "x.ai", "xai"]),
        (OpenAi, &["openai", "codex", "gpt-"]),
    ] {
        if names.iter().any(|alias| name.contains(alias)) {
            return mascot;
        }
    }
    Default
}

fn embedded_mascot(mascot: ProviderMascot, small: bool) -> Option<&'static [u8]> {
    macro_rules! pair {
        ($name:literal) => {
            if small {
                include_bytes!(concat!("../assets/providers/crew/", $name, ".small.png")).as_slice()
            } else {
                include_bytes!(concat!("../assets/providers/crew/", $name, ".png")).as_slice()
            }
        };
    }
    Some(match mascot {
        ProviderMascot::OpenAi => pair!("openai"),
        ProviderMascot::Anthropic => pair!("anthropic"),
        ProviderMascot::Google => pair!("google"),
        ProviderMascot::DeepSeek => pair!("deepseek"),
        ProviderMascot::Glm => pair!("glm"),
        ProviderMascot::Qwen => pair!("qwen"),
        ProviderMascot::Grok => pair!("grok"),
        ProviderMascot::Default => pair!("default"),
    })
}

/// The upper-bust window, in cells of the 40 cell bust: x 8..32 and y 4..28. It keeps what makes
/// a captain recognisable (Ember's goggles, Bram's cap, Jett's mohawk), and its 24 cells divide
/// a 24 pt tile exactly at 1x (1 px a cell) and 2x (2 px).
pub const BUST_WINDOW: (u32, u32, u32, u32) = (8, 4, 32, 28);

/// The cells `window` of an RGBA image drawn `k` pixels to a cell, as a new image.
fn cut_cells(
    rgba: &[u8],
    width: u32,
    k: u32,
    (x0, y0, x1, y1): (u32, u32, u32, u32),
) -> (Vec<u8>, u32, u32) {
    let (w, h) = ((x1 - x0) * k, (y1 - y0) * k);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in y0 * k..y1 * k {
        let start = ((y * width + x0 * k) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    (out, w, h)
}

pub struct Artwork {
    crew: [Set; PORTRAITS],
    pixel_art: [bool; PORTRAITS],
    mascots: HashMap<ProviderMascot, [Option<Art>; 2]>,
    /// Head crops for tiles of 24 px and below, where the full bust is too small to read.
    small: [Set; PORTRAITS],
    /// Animation strips for full busts. A captain without all four stays static.
    motion: [Option<Arc<Motion>>; PORTRAITS],
    /// KSG Code's Antigravity provider mark, a bitmap rather than an SVG.
    pub antigravity: Image,
    /// Device pixels per point, as the view last saw them (the window's scale times the app's
    /// zoom), as `f32` bits.
    scale: AtomicU32,
    /// Pre-scaled buffers by art and device size, so each is built once. They are all for the
    /// recorded scale: a new scale empties the cache, and it never holds more than
    /// `SCALED_CAP` buffers.
    scaled: Mutex<HashMap<(Shown, u32), Pixels>>,
}

/// The most pre-scaled buffers kept at once: every portrait face at a handful of sizes.
const SCALED_CAP: usize = 256;

impl Artwork {
    pub fn load(workspace: &Path) -> Arc<Self> {
        Self::load_with_root(workspace, sprite_root(workspace))
    }

    #[cfg(test)]
    pub(crate) fn load_sprite_dir(root: &Path) -> Arc<Self> {
        Self::load_with_root(root, Some(root.to_path_buf()))
    }

    fn load_with_root(workspace: &Path, root: Option<PathBuf>) -> Arc<Self> {
        let wake: Waker = Arc::new(|| {});
        let cache = ImageCache::new(wake);
        let mascot_root = workspace.join("assets/providers/crew");
        let mut mascots = HashMap::new();
        for mascot in ProviderMascot::ALL {
            let variants = [false, true].map(|small| {
                let file = format!("{}{}.png", mascot.key(), if small { ".small" } else { "" });
                root.iter()
                    .flat_map(|root| [root.join("providers/crew"), root.join("../providers/crew")])
                    .chain([mascot_root.clone()])
                    .find_map(|root| std::fs::read(root.join(&file)).ok().map(Art::new))
                    .or_else(|| embedded_mascot(mascot, small).map(Art::new))
            });
            if variants.iter().any(Option::is_some) {
                mascots.insert(mascot, variants);
            }
        }
        let mut crew = Vec::with_capacity(PORTRAITS);
        let mut small = Vec::with_capacity(PORTRAITS);
        let mut motion = Vec::with_capacity(PORTRAITS);
        let mut pixel_art = [true; PORTRAITS];
        for (captain, pixel) in pixel_art.iter_mut().enumerate() {
            let (set, crop, strips, is_pixel) = portraits(captain, root.as_deref());
            *pixel = is_pixel;
            crew.push(set);
            small.push(crop);
            motion.push(strips);
        }
        let (Ok(crew), Ok(small), Ok(motion)) = (
            <[Set; PORTRAITS]>::try_from(crew),
            <[Set; PORTRAITS]>::try_from(small),
            <[Option<Arc<Motion>>; PORTRAITS]>::try_from(motion),
        ) else {
            unreachable!("one set per portrait");
        };
        let antigravity = provider_mark(
            &cache,
            include_bytes!("../assets/providers/antigravity.png"),
        );
        // Startup only. Decode off-thread before the first frame, including headless fixtures.
        antigravity.wait().expect("valid provider mark");
        Arc::new(Self {
            crew,
            pixel_art,
            mascots,
            small,
            motion,
            antigravity,
            scale: AtomicU32::new(1.0f32.to_bits()),
            scaled: Mutex::new(HashMap::new()),
        })
    }

    /// Records the device pixels per point the view draws at. Buffers built for another scale
    /// are of no further use, so a change of scale or zoom drops them.
    pub fn set_scale(&self, scale: f32) {
        let bits = scale.max(0.25).to_bits();
        if self.scale.swap(bits, Ordering::Relaxed) != bits {
            self.scaled.lock().unwrap().clear();
        }
    }

    /// `points` in whole device pixels at the recorded scale.
    pub fn device_size(&self, points: f32) -> u32 {
        let scale = f32::from_bits(self.scale.load(Ordering::Relaxed));
        (points * scale).round().max(1.0) as u32
    }

    pub fn device_scale(&self) -> f32 {
        f32::from_bits(self.scale.load(Ordering::Relaxed))
    }

    /// The size in points closest to `points` that is a whole number of device pixels at this
    /// scale and zoom, which is the size of the buffer `device_size` asks for. Rune snaps each
    /// edge of a box to the device grid, so a slot this size keeps exactly that many pixels
    /// wherever it sits, and the art is drawn one texel to one pixel. A 28 pt slot at 110% is
    /// 30.8 pixels, which would draw as 30 or 31 and blur.
    pub fn snap_points(&self, points: f32) -> f32 {
        let scale = f32::from_bits(self.scale.load(Ordering::Relaxed));
        self.device_size(points) as f32 / scale
    }

    fn pixels_for(&self, shown: Shown, target: u32) -> Pixels {
        let mut cache = self.scaled.lock().unwrap();
        // A full cache starts over. The view asks again for what it still draws.
        if cache.len() >= SCALED_CAP && !cache.contains_key(&(shown, target)) {
            cache.clear();
        }
        cache
            .entry((shown, target))
            .or_insert_with(|| {
                let pixels = Pixels::new();
                let rgba = match shown {
                    Shown::Face(captain, face, head) => {
                        let set = if head { &self.small } else { &self.crew };
                        set[captain][face].decoded().map(|art| {
                            if self.is_pixel_art(captain) {
                                crate::crisp::fit(
                                    &art.rgba, art.width, art.height, art.cell, target,
                                )
                            } else {
                                crate::crisp::smooth(&art.rgba, art.width, art.height, target)
                            }
                        })
                    }
                    Shown::Window(captain, face) => self.crew[captain][face].decoded().map(|art| {
                        let (rgba, w, h) = cut_cells(&art.rgba, art.width, art.cell, BUST_WINDOW);
                        crate::crisp::fit(&rgba, w, h, art.cell, target)
                    }),
                    Shown::Mark(night) => {
                        let size = crate::brand::SIZE as u32;
                        Some(crate::crisp::fit(
                            &crate::brand::pixels(night),
                            size,
                            size,
                            1,
                            target,
                        ))
                    }
                    Shown::Mascot(mascot, small) => self
                        .mascot_art(mascot, small)
                        .and_then(Art::decoded)
                        .map(|art| crate::crisp::smooth(&art.rgba, art.width, art.height, target)),
                };
                // A face that failed to decode shows nothing rather than failing the view.
                pixels.set(
                    target,
                    target,
                    rgba.unwrap_or_else(|| vec![0; (target * target * 4) as usize]),
                );
                pixels
            })
            .clone()
    }

    /// A portrait's face for a tile `points` wide, scaled to whole device pixels. Tiles under
    /// 30 points use the head crop.
    pub fn face_pixels(&self, captain: usize, face: Face, points: f32) -> Pixels {
        let captain = captain.min(PORTRAITS - 1);
        self.pixels_for(
            Shown::Face(captain, face.index(), points < 30.0),
            self.device_size(points),
        )
    }

    /// The upper bust (`BUST_WINDOW`) for a tile `points` wide, in whole device pixels when
    /// `points` times the scale is a multiple of 24.
    pub fn bust_window_pixels(&self, captain: usize, face: Face, points: f32) -> Pixels {
        let captain = captain.min(PORTRAITS - 1);
        if !self.is_pixel_art(captain) {
            return self.face_pixels(captain, face, points);
        }
        self.pixels_for(
            Shown::Window(captain, face.index()),
            self.device_size(points),
        )
    }

    /// The app mark for a square `points` wide. `night` selects the dark-theme sail.
    pub fn mark_pixels(&self, night: bool, points: f32) -> Pixels {
        self.pixels_for(Shown::Mark(night), self.device_size(points))
    }

    /// A captain's animation strips, if its set has them.
    pub fn motion(&self, captain: usize) -> Option<&Motion> {
        self.motion.get(captain)?.as_deref()
    }

    pub fn is_pixel_art(&self, captain: usize) -> bool {
        self.pixel_art[captain.min(PORTRAITS - 1)]
    }

    fn mascot_art(&self, mascot: ProviderMascot, small: bool) -> Option<&Art> {
        let variants = self.mascots.get(&mascot)?;
        [usize::from(small), usize::from(!small)]
            .into_iter()
            .filter_map(|index| variants[index].as_ref())
            .find(|art| art.decoded().is_some())
    }

    pub fn mascot_pixels(&self, mascot: ProviderMascot, points: f32) -> Option<Pixels> {
        let small = points < 30.0;
        self.mascot_art(mascot, small)?;
        Some(self.pixels_for(Shown::Mascot(mascot, small), self.device_size(points)))
    }
}

/// An explicit drop-in folder wins over the workspace folder. Neither is required.
fn sprite_root(workspace: &Path) -> Option<PathBuf> {
    env::var_os("SHINA_SPRITES")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| {
            let local = workspace.join("assets").join("crew");
            local.is_dir().then_some(local)
        })
        .or_else(|| {
            let packaged = PathBuf::from("assets/crew");
            packaged.is_dir().then_some(packaged)
        })
}

fn pixel_set(set: &Set) -> bool {
    set[Face::Normal.index()].decoded().is_some_and(|art| {
        art.width == art.height
            && (art.width == 40
                || art.width == 240
                    && crate::crisp::nearest_grid(art.width, art.height, &art.rgba, 6))
    })
}

fn portraits(captain: usize, root: Option<&Path>) -> (Set, Set, Option<Arc<Motion>>, bool) {
    if let Some((set, small, folder)) = root.and_then(|root| load_set(root, captain)) {
        // User-provided files must be read again for each new app state.
        let pixel = pixel_set(&set);
        return (
            set,
            small,
            pixel.then(|| load_motion(&folder, captain)).flatten(),
            pixel,
        );
    }
    let (set, small) = embedded(captain);
    let pixel = pixel_set(&set);
    (
        set,
        small,
        pixel.then(|| embedded_motion(captain)).flatten(),
        pixel,
    )
}

/// All four strips from a drop-in folder, or none: a partial set would animate some states
/// and not others.
fn load_motion(folder: &Path, captain: usize) -> Option<Arc<Motion>> {
    let pngs: Vec<Vec<u8>> = Clip::ALL
        .iter()
        .map(|clip| std::fs::read(folder.join(format!("{}.strip.png", clip.file()))).ok())
        .collect::<Option<_>>()?;
    // A dev checkout contains the embedded files too. Compare fresh bytes so
    // unchanged files reuse their decodes, without hiding edits to custom art.
    if pngs
        .iter()
        .zip(embedded_motion_pngs(captain))
        .all(|(disk, embedded)| disk == embedded)
    {
        return embedded_motion(captain);
    }
    decode_motion(pngs.iter().map(Vec::as_slice)).map(Arc::new)
}

fn embedded_motion(captain: usize) -> Option<Arc<Motion>> {
    // Only embedded bytes are immutable. Do not share Artwork itself: window
    // scale, scaled buffers and portrait playback belong to each app state.
    static MOTION: [OnceLock<Option<Arc<Motion>>>; PORTRAITS] =
        [const { OnceLock::new() }; PORTRAITS];
    MOTION[captain.min(PORTRAITS - 1)]
        .get_or_init(|| decode_motion(embedded_motion_pngs(captain)).map(Arc::new))
        .clone()
}

fn embedded_motion_pngs(captain: usize) -> [&'static [u8]; 4] {
    macro_rules! strips {
        ($name:literal) => {
            [
                include_bytes!(concat!("../assets/crew-pixel/", $name, "/Idle.strip.png"))
                    .as_slice(),
                include_bytes!(concat!(
                    "../assets/crew-pixel/",
                    $name,
                    "/Working.strip.png"
                ))
                .as_slice(),
                include_bytes!(concat!(
                    "../assets/crew-pixel/",
                    $name,
                    "/Attention.strip.png"
                ))
                .as_slice(),
                include_bytes!(concat!("../assets/crew-pixel/", $name, "/Happy.strip.png"))
                    .as_slice(),
            ]
        };
    }
    match captain {
        0 => strips!("ember"),
        1 => strips!("juniper"),
        2 => strips!("sable"),
        3 => strips!("pip"),
        4 => strips!("bram"),
        5 => strips!("wren"),
        6 => strips!("rook"),
        7 => strips!("nia"),
        8 => strips!("jett"),
        9 => strips!("otto"),
        10 => strips!("bolt"),
        _ => strips!("kit"),
    }
}

fn decode_motion<'a>(pngs: impl IntoIterator<Item = &'a [u8]>) -> Option<Motion> {
    let strips: Vec<Strip> = Clip::ALL
        .iter()
        .zip(pngs)
        .map(|(clip, png)| Strip::decode(png, *clip))
        .collect::<Option<_>>()?;
    Motion::try_from(strips).ok()
}

/// A named or numeric drop-in set needs only that captain's Normal face.
fn load_set(root: &Path, captain: usize) -> Option<(Set, Set, PathBuf)> {
    let folders = [
        root.join(CAPTAIN_KEYS[captain]),
        root.join(captain.to_string()),
    ];
    for folder in folders {
        if !folder.is_dir() {
            continue;
        }
        let (embedded_faces, embedded_crops) = embedded(captain);
        let load = |face: usize, small: bool| {
            let suffix = if small { ".small.png" } else { ".png" };
            let bytes = std::fs::read(folder.join(format!("{}{suffix}", FACE_FILES[face]))).ok()?;
            let embedded = if small {
                &embedded_crops[face]
            } else {
                &embedded_faces[face]
            };
            let art = if bytes.as_slice() == embedded.png.as_ref() {
                embedded.clone()
            } else {
                Art::new(bytes)
            };
            art.decoded()?;
            Some(art)
        };
        let normal_index = Face::Normal.index();
        let Some(normal) = load(normal_index, false) else {
            continue;
        };
        let normal_small = load(normal_index, true).unwrap_or_else(|| normal.clone());
        let faces: [Option<Art>; Face::COUNT] = std::array::from_fn(|face| {
            if face == normal_index {
                Some(normal.clone())
            } else {
                load(face, false)
            }
        });
        let small = std::array::from_fn(|face| {
            if face == normal_index {
                normal_small.clone()
            } else {
                load(face, true)
                    .or_else(|| faces[face].clone())
                    .unwrap_or_else(|| normal_small.clone())
            }
        });
        return Some((
            faces.map(|face| face.unwrap_or_else(|| normal.clone())),
            small,
            folder,
        ));
    }
    None
}

fn embedded(captain: usize) -> (Set, Set) {
    static FACES: [OnceLock<(Set, Set)>; PORTRAITS] = [const { OnceLock::new() }; PORTRAITS];
    FACES[captain.min(PORTRAITS - 1)]
        .get_or_init(|| embedded_faces(captain))
        .clone()
}

fn embedded_faces(captain: usize) -> (Set, Set) {
    macro_rules! set {
        ($name:literal) => {{
            let full = Art::new(
                include_bytes!(concat!("../assets/crew/", $name, "/Normal.png")).as_slice(),
            );
            let small = Art::new(
                include_bytes!(concat!("../assets/crew/", $name, "/Normal.small.png")).as_slice(),
            );
            // The approved artwork has no alternate expressions. Share the same decode
            // for every status; the halo and status mark still show the live state.
            (
                std::array::from_fn(|_| full.clone()),
                std::array::from_fn(|_| small.clone()),
            )
        }};
    }
    match captain {
        0 => set!("ember"),
        1 => set!("juniper"),
        2 => set!("sable"),
        3 => set!("pip"),
        4 => set!("bram"),
        5 => set!("wren"),
        6 => set!("rook"),
        7 => set!("nia"),
        8 => set!("jett"),
        9 => set!("otto"),
        10 => set!("bolt"),
        _ => set!("kit"),
    }
}

/// A provider mark from PNG bytes.
fn provider_mark(cache: &ImageCache, png: &[u8]) -> Image {
    let image = image::load_from_memory(png)
        .expect("valid provider mark")
        .to_rgba8();
    cache.rgba(image.width(), image.height(), image.into_raw())
}

/// The colour behind each portrait: the corner pixel of its neutral face. A captain's horizon
/// band takes its sky from this.
pub fn portrait_backdrop(portrait: usize) -> [u8; 3] {
    static BACKDROPS: std::sync::OnceLock<[[u8; 3]; PORTRAITS]> = std::sync::OnceLock::new();
    macro_rules! faces {
        ($($name:literal),*) => {
            [$(include_bytes!(concat!("../assets/crew/", $name, "/Normal.png")).as_slice()),*]
        };
    }
    let backdrops = BACKDROPS.get_or_init(|| {
        let faces: [&[u8]; PORTRAITS] = faces!(
            "ember", "juniper", "sable", "pip", "bram", "wren", "rook", "nia", "jett", "otto",
            "bolt", "kit"
        );
        faces.map(|bytes| {
            image::load_from_memory(bytes)
                .map(|face| {
                    let [r, g, b, _] = face.to_rgba8().get_pixel(1, 1).0;
                    [r, g, b]
                })
                .unwrap_or([0x1d, 0x2a, 0x45])
        })
    });
    backdrops[portrait.min(PORTRAITS - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_expression_falls_back_to_the_same_captains_normal() {
        let workspace = crate::store::directory().unwrap();
        let folder = workspace.join("assets/crew/ember");
        std::fs::create_dir_all(&folder).unwrap();
        image::RgbaImage::from_fn(512, 512, |x, y| image::Rgba([x as u8, y as u8, 150, 255]))
            .save(folder.join("Normal.png"))
            .unwrap();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([0, 129, 201, 255]))
            .save(folder.join("Normal.small.png"))
            .unwrap();
        let (faces, crops, motion, pixel) = portraits(0, folder.parent());
        assert!(!pixel);
        assert!(motion.is_none());
        assert!(Arc::ptr_eq(
            &faces[Face::Happy.index()].png,
            &faces[Face::Normal.index()].png
        ));
        assert!(Arc::ptr_eq(
            &crops[Face::Happy.index()].png,
            &crops[Face::Normal.index()].png
        ));
        let art = Artwork::load(&workspace);
        assert!(!art.is_pixel_art(0));
        art.set_scale(1.25);
        for size in [16.0, 24.0, 36.0, 48.0, 96.0, 160.0] {
            art.face_pixels(0, Face::Happy, size);
            assert!(art.scaled.lock().unwrap().contains_key(&(
                Shown::Face(0, Face::Happy.index(), size < 30.0),
                art.device_size(size)
            )));
        }
    }

    #[test]
    fn provider_mascot_aliases_follow_the_model_not_its_transport() {
        for (name, provider) in [
            ("openrouter/DeepSeek-R1", ProviderMascot::DeepSeek),
            ("gpt-6.2-sol", ProviderMascot::OpenAi),
            ("Anthropic Claude", ProviderMascot::Anthropic),
            ("gemini-4-neon", ProviderMascot::Google),
            ("z.ai/glm-5.3", ProviderMascot::Glm),
            ("Qwen-Coder", ProviderMascot::Qwen),
            ("cursor-grok-4.6", ProviderMascot::Grok),
            ("custom-local", ProviderMascot::Default),
        ] {
            assert_eq!(provider_mascot_for(name), provider);
        }
    }

    #[test]
    fn all_embedded_captains_and_provider_mascots_ship_as_illustrated_sets() {
        // Skip the checkout's assets directory and any SHINA_SPRITES override. Released
        // clients and servers must be able to load the set from the binary alone.
        let workspace = crate::store::directory().unwrap();
        let art = Artwork::load_with_root(&workspace, None);
        for captain in 0..PORTRAITS {
            assert!(
                !art.is_pixel_art(captain),
                "captain {captain} should be illustrated"
            );
            assert!(
                art.motion(captain).is_none(),
                "illustrated captain {captain} should not play pixel strips"
            );
            for face in 0..Face::COUNT {
                let full = art.crew[captain][face].decoded().expect("full face PNG");
                let small = art.small[captain][face].decoded().expect("small face PNG");
                assert_eq!((full.width, full.height), (512, 512));
                assert_eq!((small.width, small.height), (256, 256));
                assert!(Arc::ptr_eq(
                    &art.crew[captain][face].decoded,
                    &art.crew[captain][Face::Normal.index()].decoded
                ));
                assert!(Arc::ptr_eq(
                    &art.small[captain][face].decoded,
                    &art.small[captain][Face::Normal.index()].decoded
                ));
            }
        }
        for mascot in ProviderMascot::ALL {
            let full = art
                .mascot_art(mascot, false)
                .and_then(Art::decoded)
                .expect("embedded mascot 512");
            let small = art
                .mascot_art(mascot, true)
                .and_then(Art::decoded)
                .expect("embedded mascot 256");
            assert_eq!((full.width, full.height), (512, 512));
            assert_eq!((small.width, small.height), (256, 256));
            assert!(art.mascot_pixels(mascot, 36.0).is_some());
            assert!(art.mascot_pixels(mascot, 20.0).is_some());
        }
    }
}
