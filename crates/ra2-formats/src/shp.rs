//! SHP sprites (Tiberian Sun / Red Alert 2 variant): palette-indexed frames, each
//! cropped to its own rectangle within the sprite's full canvas.

pub struct Frame {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// `width * height` palette indices, row by row. Index 0 is transparent.
    pub pixels: Vec<u8>,
    /// The colour the frame shows as on the radar.
    pub radar: [u8; 3],
}

pub struct Shp {
    pub width: u16,
    pub height: u16,
    pub frames: Vec<Frame>,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

impl Shp {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if u16_at(data, 0)? != 0 {
            return None;
        }
        let width = u16_at(data, 2)?;
        let height = u16_at(data, 4)?;
        let count = u16_at(data, 6)? as usize;
        let mut frames = Vec::with_capacity(count);
        for index in 0..count {
            let header = data.get(8 + index * 24..8 + (index + 1) * 24)?;
            let w = u16_at(header, 4)?;
            let h = u16_at(header, 6)?;
            let flags = header[8];
            let offset = u32::from_le_bytes(header[20..24].try_into().ok()?) as usize;
            let pixels = if offset == 0 || w == 0 || h == 0 {
                vec![0; w as usize * h as usize]
            } else {
                decode_frame(data.get(offset..)?, w as usize, h as usize, flags)?
            };
            frames.push(Frame {
                x: u16_at(header, 0)?,
                y: u16_at(header, 2)?,
                width: w,
                height: h,
                pixels,
                radar: [header[12], header[13], header[14]],
            });
        }
        Some(Self {
            width,
            height,
            frames,
        })
    }

    /// One frame expanded to the full canvas as RGBA.
    pub fn frame_rgba(&self, index: usize, palette: &crate::pal::Palette) -> Vec<u8> {
        let (cw, ch) = (self.width as usize, self.height as usize);
        let mut out = vec![0u8; cw * ch * 4];
        let frame = &self.frames[index];
        for row in 0..frame.height as usize {
            for col in 0..frame.width as usize {
                let color = frame.pixels[row * frame.width as usize + col];
                let (x, y) = (frame.x as usize + col, frame.y as usize + row);
                if color != 0 && x < cw && y < ch {
                    let [r, g, b] = palette.0[color as usize];
                    out[(y * cw + x) * 4..][..4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
        out
    }
}

fn decode_frame(data: &[u8], w: usize, h: usize, flags: u8) -> Option<Vec<u8>> {
    // Bit 1: each scanline is prefixed with its byte length.
    // Bit 2: within a scanline, a zero byte is followed by a run length of zeros.
    if flags & 2 == 0 {
        return Some(data.get(..w * h)?.to_vec());
    }
    let mut pixels = vec![0u8; w * h];
    let mut at = 0;
    for row in pixels.chunks_exact_mut(w) {
        let len = u16_at(data, at)? as usize;
        let line = data.get(at + 2..at + len)?;
        at += len;
        if flags & 1 == 0 {
            let n = line.len().min(w);
            row[..n].copy_from_slice(&line[..n]);
            continue;
        }
        let mut x = 0;
        let mut i = 0;
        while i < line.len() && x < w {
            if line[i] == 0 {
                x += *line.get(i + 1)? as usize;
                i += 2;
            } else {
                row[x] = line[i];
                x += 1;
                i += 1;
            }
        }
    }
    Some(pixels)
}
