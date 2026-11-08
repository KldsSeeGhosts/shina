//! Status portraits for the three crew captains.
//!
//! The Pokémon Mystery Dungeon portraits come from PMDCollab's SpriteCollab and are
//! licensed CC BY-NC 4.0. They are included for this local, non-commercial experiment
//! and are not covered by the source code's MIT declaration. See
//! `assets/sprites/README.md` for credits and redistribution terms.
use rune::{Image, ImageCache, Waker};
use std::sync::Arc;

/// Display names, in the same order as the captain indices used by threads.
pub const CAPTAIN_NAMES: [&str; 3] = ["Bulbasaur", "Charmander", "Squirtle"];
/// The work each captain is responsible for.
pub const CAPTAIN_ROLES: [&str; 3] = ["Platform", "Release", "Support"];
pub const CAPTAIN_ROLE_DETAIL: [&str; 3] = [
    "Architecture and implementation",
    "Release planning and validation",
    "Support investigations and fixes",
];

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
    pub const COUNT: usize = 6;
    pub fn index(self) -> usize {
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

pub struct Artwork {
    crew: [[Image; Face::COUNT]; 3],
    pub mark: Image,
}

impl Artwork {
    pub fn load() -> Arc<Self> {
        let wake: Waker = Arc::new(|| {});
        let cache = ImageCache::new(wake);
        macro_rules! portraits {
            ($mon:literal) => {
                [
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Normal.png")),
                    ),
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Happy.png")),
                    ),
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Determined.png")),
                    ),
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Surprised.png")),
                    ),
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Dizzy.png")),
                    ),
                    portrait(
                        &cache,
                        include_bytes!(concat!("../assets/sprites/pmd/", $mon, "/Sad.png")),
                    ),
                ]
            };
        }
        let crew = [
            portraits!("bulbasaur"),
            portraits!("charmander"),
            portraits!("squirtle"),
        ];
        let mark = mark(&cache);
        // Startup only. Decode off-thread before the first frame, including headless fixtures.
        for image in crew.iter().flatten().chain(std::iter::once(&mark)) {
            image.wait().expect("valid embedded crew artwork");
        }
        Arc::new(Self { crew, mark })
    }

    pub fn face(&self, captain: usize, face: Face) -> &Image {
        &self.crew[captain.min(2)][face.index()]
    }
}

fn portrait(cache: &ImageCache, bytes: &[u8]) -> Image {
    cache.load(bytes)
}

/// A small original pixel mark: a dark crew badge with a cream "S" and three service dots.
fn mark(cache: &ImageCache) -> Image {
    const SIZE: usize = 24;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let mut put = |x: usize, y: usize, color: [u8; 4]| {
        let at = (y * SIZE + x) * 4;
        rgba[at..at + 4].copy_from_slice(&color);
    };
    let ink = [35, 35, 31, 255];
    let cream = [244, 239, 227, 255];
    let green = [143, 188, 106, 255];
    let orange = [224, 122, 84, 255];
    let blue = [106, 168, 200, 255];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let corner_x = x.clamp(5, SIZE - 6);
            let corner_y = y.clamp(5, SIZE - 6);
            let dx = x as i32 - corner_x as i32;
            let dy = y as i32 - corner_y as i32;
            if dx * dx + dy * dy <= 25 {
                put(x, y, ink);
            }
        }
    }
    const S: [&str; 7] = [
        "01110", "10000", "10000", "01110", "00001", "00001", "01110",
    ];
    for (row, bits) in S.iter().enumerate() {
        for (column, bit) in bits.bytes().enumerate() {
            if bit != b'0' {
                for dy in 0..2 {
                    for dx in 0..2 {
                        put(7 + column * 2 + dx, 5 + row * 2 + dy, cream);
                    }
                }
            }
        }
    }
    for (offset, color) in [(7, green), (11, orange), (15, blue)] {
        put(offset, 20, color);
        put(offset + 1, 20, color);
        put(offset, 21, color);
        put(offset + 1, 21, color);
    }
    let image = cache.rgba(SIZE as u32, SIZE as u32, rgba);
    image.wait().expect("valid generated crew mark");
    image
}
