//! TMP isometric terrain tiles. A file is a template of one or more cells; each cell
//! has a diamond-shaped image and optionally an "extra" rectangle for anything that
//! rises above the diamond, such as a cliff face.

pub struct Extra {
    /// Position relative to the cell's diamond bounding box.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// `width * height` palette indices; 0 is transparent.
    pub pixels: Vec<u8>,
    /// Depth of each pixel, where the file has it.
    pub z: Option<Vec<u8>>,
}

pub struct Cell {
    pub height: u8,
    pub terrain_type: u8,
    pub ramp_type: u8,
    /// Whether the cell carries a second, damaged image (used by bridge pieces).
    pub has_damaged_data: bool,
    /// The colour the cell shows as on the radar (the header's "left" colour).
    pub radar: [u8; 3],
    /// The diamond expanded to `width * height` palette indices. Pixels outside the
    /// diamond are marked in `mask`.
    pub pixels: Vec<u8>,
    /// Depth of each pixel of the diamond, laid out as `pixels`, where the file
    /// has it.
    // Header flag 2 says the tile has depth data, at
    // the header's for the diamond and for the extra part.
    pub z: Option<Vec<u8>>,
    pub extra: Option<Extra>,
}

pub struct Tmp {
    /// Template size in cells.
    pub blocks_x: u32,
    pub blocks_y: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    /// One entry per template position; `None` where the template has a hole.
    pub cells: Vec<Option<Cell>>,
}

const CELL_HEADER_LEN: usize = 52;
const HAS_EXTRA: u32 = 1;
const HAS_Z_DATA: u32 = 2;
const HAS_DAMAGED_DATA: u32 = 4;

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// The horizontal span `(start, length)` of the diamond on one row.
pub fn diamond_row(row: u32, width: u32, height: u32) -> (u32, u32) {
    let half = height / 2;
    // Rows widen by 4 px down to the middle, then narrow again; the last row is empty.
    let steps = if row < half { row + 1 } else { height - 1 - row };
    let length = steps * (width / half);
    (width / 2 - length / 2, length)
}

impl Tmp {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let blocks_x = u32_at(data, 0)? as usize;
        let blocks_y = u32_at(data, 4)? as usize;
        let cell_width = u32_at(data, 8)?;
        let cell_height = u32_at(data, 12)?;
        if cell_width == 0 || cell_height < 2 || cell_width > 256 || cell_height > 256 {
            return None;
        }
        let count = blocks_x.checked_mul(blocks_y)?;
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            let offset = u32_at(data, 16 + index * 4)? as usize;
            if offset == 0 {
                cells.push(None);
                continue;
            }
            let header = data.get(offset..offset + CELL_HEADER_LEN)?;
            let cell_x = u32_at(header, 0)? as i32;
            let cell_y = u32_at(header, 4)? as i32;
            let flags = u32_at(header, 36)?;

            let mut pixels = vec![0u8; (cell_width * cell_height) as usize];
            let mut at = offset + CELL_HEADER_LEN;
            for row in 0..cell_height {
                let (start, length) = diamond_row(row, cell_width, cell_height);
                let source = data.get(at..at + length as usize)?;
                let begin = (row * cell_width + start) as usize;
                pixels[begin..begin + length as usize].copy_from_slice(source);
                at += length as usize;
            }

            let extra = if flags & HAS_EXTRA != 0 {
                let width = u32_at(header, 28)?;
                let height = u32_at(header, 32)?;
                let start = offset + u32_at(header, 8)? as usize;
                let len = (width as usize).checked_mul(height as usize)?;
                Some(Extra {
                    x: u32_at(header, 20)? as i32 - cell_x,
                    y: u32_at(header, 24)? as i32 - cell_y,
                    width,
                    height,
                    pixels: data.get(start..start + len)?.to_vec(),
                    z: (flags & HAS_Z_DATA != 0)
                        .then(|| {
                            let start = offset + u32_at(header, 16)? as usize;
                            data.get(start..start + len).map(<[u8]>::to_vec)
                        })
                        .flatten(),
                })
            } else {
                None
            };

            let z = (flags & HAS_Z_DATA != 0)
                .then(|| {
                    let mut z = vec![0u8; (cell_width * cell_height) as usize];
                    let mut at = offset + u32_at(header, 12)? as usize;
                    for row in 0..cell_height {
                        let (start, length) = diamond_row(row, cell_width, cell_height);
                        let source = data.get(at..at + length as usize)?;
                        let begin = (row * cell_width + start) as usize;
                        z[begin..begin + length as usize].copy_from_slice(source);
                        at += length as usize;
                    }
                    Some(z)
                })
                .flatten();
            cells.push(Some(Cell {
                z,
                height: header[40],
                terrain_type: header[41],
                ramp_type: header[42],
                has_damaged_data: flags & HAS_DAMAGED_DATA != 0,
                radar: [header[0x2b], header[0x2c], header[0x2d]],
                pixels,
                extra,
            }));
        }
        Some(Self {
            blocks_x: blocks_x as u32,
            blocks_y: blocks_y as u32,
            cell_width,
            cell_height,
            cells,
        })
    }
}
