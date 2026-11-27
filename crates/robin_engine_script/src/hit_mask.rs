//! Pixel-level hit masks prepared from interface sprites. `robin_engine::minimap`
//! re-exports the type.

use serde::{Deserialize, Serialize};

/// Pre-computed bitmask of non-transparent pixels for a minimap surface.
///
/// We pre-compute the mask at load time so click-hit checks don't have to
/// lock the surface and read a pixel each frame.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct HitMask {
    width: u16,
    height: u16,
    /// One bool per pixel — `true` = opaque (hit), `false` = transparent (miss).
    opaque: Vec<bool>,
}

impl HitMask {
    /// Build a hit mask from raw 16-bit pixel data.
    pub fn from_pixels_u16(
        width: u16,
        height: u16,
        pixels: &[u16],
        transparent_color: u16,
    ) -> Self {
        let opaque = pixels.iter().map(|&px| px != transparent_color).collect();
        Self {
            width,
            height,
            opaque,
        }
    }

    /// Build a hit mask from exported opacity bits, rejecting malformed payloads.
    pub fn from_opacity(width: u16, height: u16, opaque: Vec<bool>) -> Result<Self, &'static str> {
        if opaque.len() != usize::from(width) * usize::from(height) {
            return Err("hit mask length disagrees with dimensions");
        }
        Ok(Self {
            width,
            height,
            opaque,
        })
    }

    /// Check if the pixel at `(x, y)` is opaque (non-transparent).
    pub fn is_opaque(&self, x: u16, y: u16) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.opaque[y as usize * self.width as usize + x as usize]
    }
}
