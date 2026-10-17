//! PCX images, used for dialog widgets. Only the 8-bit paletted and 24-bit RLE
//! variants the game ships are handled.

pub struct Pcx {
    pub width: u32,
    pub height: u32,
    /// `width * height` RGB triples.
    pub rgb: Vec<u8>,
}

impl Pcx {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 128 || data[0] != 0x0A || data[3] != 8 {
            return None;
        }
        let word = |at: usize| u16::from_le_bytes([data[at], data[at + 1]]) as usize;
        let width = word(8).checked_sub(word(4))? + 1;
        let height = word(10).checked_sub(word(6))? + 1;
        let planes = data[65] as usize;
        let stride = word(66);
        if stride < width || !(planes == 1 || planes == 3) {
            return None;
        }

        let mut raw = Vec::with_capacity(stride * planes * height);
        let mut at = 128;
        while raw.len() < stride * planes * height {
            let byte = *data.get(at)?;
            at += 1;
            if byte & 0xC0 == 0xC0 {
                let value = *data.get(at)?;
                at += 1;
                raw.extend(std::iter::repeat_n(value, (byte & 0x3F) as usize));
            } else {
                raw.push(byte);
            }
        }

        let mut rgb = Vec::with_capacity(width * height * 3);
        if planes == 1 {
            let palette = data.get(data.len().checked_sub(768)?..)?;
            for row in raw.chunks_exact(stride).take(height) {
                for &index in &row[..width] {
                    rgb.extend_from_slice(&palette[index as usize * 3..][..3]);
                }
            }
        } else {
            for row in raw.chunks_exact(stride * 3).take(height) {
                for x in 0..width {
                    rgb.extend_from_slice(&[row[x], row[stride + x], row[stride * 2 + x]]);
                }
            }
        }
        Some(Self {
            width: width as u32,
            height: height as u32,
            rgb,
        })
    }
}
