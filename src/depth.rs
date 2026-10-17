//! Depth against the ground: which pixels of a unit's picture the terrain hides.
//!
//! Follows the original game
//! The engine keeps a depth for every pixel on screen; terrain tiles write it and
//! units are tested against it, so a unit behind a cliff is cut off exactly where
//! the cliff's picture covers it. Here the same numbers are worked out on demand
//! for the pixels of one unit's picture.
//!
//! Depths are in the engine's units with its scrolling constant taken as the
//! scene's own row 0, so that a flat ground pixel on scene row r at level 0 has
//! depth -r - 1. Smaller is nearer.
//!

use std::collections::HashMap;

use bevy::prelude::*;

/// The depth data of one tile picture.
pub struct DepthTile {
    pub width: i32,
    pub height: i32,
    /// For each pixel, its depth byte, or `None` where the picture is empty.
    pub z: Vec<Option<u8>>,
}

/// A tile picture placed on the map.
struct Placed {
    x: i32,
    y: i32,
    /// The scene row of the top of the cell's diamond were it at level 0.
    flat_top: i32,
    level: i32,
    tile: usize,
    raised: bool,
}

const BUCKET: i32 = 64;

#[derive(Resource, Default)]
pub struct TerrainDepth {
    tiles: Vec<DepthTile>,
    placed: Vec<Placed>,
    buckets: HashMap<(i32, i32), Vec<u32>>,
}

impl TerrainDepth {
    pub fn add_tile(&mut self, tile: DepthTile) -> usize {
        self.tiles.push(tile);
        self.tiles.len() - 1
    }

    /// Records a tile picture drawn with its corner at a scene pixel. `flat_top`
    /// is the scene row of the top of the cell's diamond at level 0; `raised`
    /// marks the part of a tile that stands up out of its diamond.
    pub fn place(&mut self, tile: usize, x: i32, y: i32, flat_top: i32, level: i32, raised: bool) {
        let Some(shape) = self.tiles.get(tile) else {
            return;
        };
        let id = self.placed.len() as u32;
        for by in y.div_euclid(BUCKET)..=(y + shape.height - 1).div_euclid(BUCKET) {
            for bx in x.div_euclid(BUCKET)..=(x + shape.width - 1).div_euclid(BUCKET) {
                self.buckets.entry((bx, by)).or_default().push(id);
            }
        }
        self.placed.push(Placed { x, y, flat_top, level, tile, raised });
    }

    /// Which pixels of a picture the ground hides: the picture's corner and size
    /// in scene pixels, the level its owner stands on, and its depth on each
    /// scene row. Returns `None` when nothing is hidden.
    // A tile pixel's depth is that of the top of its cell's box at level 0, less
    // 30, plus the pixel's depth byte; it is written where it is no further than
    // what is there. A tested pixel is drawn only where its depth is less than
    // the one stored.
    pub fn hidden(&self, x: i32, y: i32, width: i32, height: i32, level: i32, depth: impl Fn(i32) -> i32) -> Option<Vec<bool>> {
        let mut seen: Vec<u32> = Vec::new();
        for by in y.div_euclid(BUCKET)..=(y + height - 1).div_euclid(BUCKET) {
            for bx in x.div_euclid(BUCKET)..=(x + width - 1).div_euclid(BUCKET) {
                for &id in self.buckets.get(&(bx, by)).map_or(&[][..], Vec::as_slice) {
                    // Flat ground at or below the owner's own level cannot be
                    // nearer than something standing on it.
                    let placed = &self.placed[id as usize];
                    if (placed.raised || placed.level > level) && !seen.contains(&id) {
                        seen.push(id);
                    }
                }
            }
        }
        if seen.is_empty() {
            return None;
        }
        let mut mask = vec![false; (width * height) as usize];
        let mut any = false;
        for id in seen {
            let placed = &self.placed[id as usize];
            let tile = &self.tiles[placed.tile];
            let base = -placed.flat_top - 30;
            let (x0, x1) = (x.max(placed.x), (x + width).min(placed.x + tile.width));
            let (y0, y1) = (y.max(placed.y), (y + height).min(placed.y + tile.height));
            for row in y0..y1 {
                let mine = depth(row);
                for col in x0..x1 {
                    let Some(z) = tile.z[((row - placed.y) * tile.width + (col - placed.x)) as usize] else {
                        continue;
                    };
                    if base + z as i32 <= mine {
                        mask[((row - y) * width + (col - x)) as usize] = true;
                        any = true;
                    }
                }
            }
        }
        any.then_some(mask)
    }
}

/// The depth of an upright picture (a unit, a soldier) on a scene row.
// From the bottom row of the picture's own
// rectangle and its adjustment, rounded down to a multiple of three, getting
// nearer by one for every three rows up.
pub fn upright(bottom: i32, adjust: i32, row: i32) -> i32 {
    3 * (-bottom + adjust).div_euclid(3) - (bottom - row).div_euclid(3)
}
