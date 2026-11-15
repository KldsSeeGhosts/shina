//! Ephemeral desktop QR image. Pairing credentials never enter saved UI state.
use crate::mobile_pairing::PairLink;
use std::sync::Mutex;

pub struct Share {
    pub link: PairLink,
    cells: u32,
    source: Vec<bool>,
    rendered: Mutex<Option<(u32, rune::Pixels)>>,
}

impl Share {
    pub fn new(link: PairLink) -> Result<Self, String> {
        let code = qrcode::QrCode::new(link.encode().as_bytes())
            .map_err(|_| "Could not encode the mobile pairing QR.")?;
        let width = code.width() as u32;
        let cells = width + 8;
        let mut source = vec![false; (cells * cells) as usize];
        for y in 0..width {
            for x in 0..width {
                source[((y + 4) * cells + x + 4) as usize] =
                    code[(x as usize, y as usize)] == qrcode::Color::Dark;
            }
        }
        Ok(Self {
            link,
            cells,
            source,
            rendered: Mutex::new(None),
        })
    }

    /// Each module occupies whole device pixels. Rune receives a buffer with
    /// exactly the device dimensions it draws, including a four-module quiet zone.
    pub fn pixels(&self, scale: f32) -> (rune::Pixels, f32) {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale.clamp(0.25, 8.0)
        } else {
            1.0
        };
        let factor = (168.0 * scale / self.cells as f32).floor().max(1.0) as u32;
        let size = self.cells * factor;
        let mut rendered = self.rendered.lock().expect("mobile QR cache");
        if !rendered.as_ref().is_some_and(|(old, _)| *old == size) {
            let mut bytes = Vec::with_capacity((size * size * 4) as usize);
            for y in 0..size {
                for x in 0..size {
                    let dark = self.source[((y / factor) * self.cells + x / factor) as usize];
                    bytes.extend_from_slice(if dark {
                        &[0, 0, 0, 255]
                    } else {
                        &[255, 255, 255, 255]
                    });
                }
            }
            let pixels = rune::Pixels::new();
            pixels.set(size, size, bytes);
            *rendered = Some((size, pixels));
        }
        (rendered.as_ref().unwrap().1.clone(), size as f32 / scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share() -> Share {
        Share::new(
            PairLink::new(
                "100.90.1.2",
                47329,
                uuid::Uuid::nil(),
                1_800_000_000_000,
                "012345ABCDEF",
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn qr_has_four_quiet_modules_and_a_whole_pixel_scale() {
        let share = share();
        for y in 0..share.cells {
            for x in 0..share.cells {
                if x < 4 || y < 4 || x >= share.cells - 4 || y >= share.cells - 4 {
                    assert!(!share.source[(y * share.cells + x) as usize]);
                }
            }
        }
        for scale in [0.5, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0] {
            let (_, points) = share.pixels(scale);
            let device = (points * scale).round() as u32;
            assert_eq!(device % share.cells, 0);
            assert!(device > 0);
            assert_eq!(share.rendered.lock().unwrap().as_ref().unwrap().0, device);
        }
    }
}
