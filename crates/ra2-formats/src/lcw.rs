//! LCW ("Format80") decompression, Westwood's LZ variant.

/// Decompresses `src`, producing at most `max_len` bytes.
pub fn decompress(src: &[u8], max_len: usize) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(max_len);
    let mut at = 0;
    let byte = |at: &mut usize| -> Option<usize> {
        let value = *src.get(*at)? as usize;
        *at += 1;
        Some(value)
    };
    let word = |at: &mut usize| -> Option<usize> {
        let value = u16::from_le_bytes([*src.get(*at)?, *src.get(*at + 1)?]) as usize;
        *at += 2;
        Some(value)
    };
    // Copies may overlap their own output, so they go byte by byte.
    fn copy(out: &mut Vec<u8>, from: usize, count: usize) -> Option<()> {
        for i in 0..count {
            let value = *out.get(from + i)?;
            out.push(value);
        }
        Some(())
    }

    loop {
        let command = byte(&mut at)?;
        if command & 0x80 == 0 {
            // Short copy from a position relative to the end of the output.
            let count = (command >> 4) + 3;
            let back = ((command & 0x0F) << 8) | byte(&mut at)?;
            let from = out.len().checked_sub(back)?;
            copy(&mut out, from, count)?;
        } else if command & 0x40 == 0 {
            // Literal run; a zero-length run ends the stream.
            let count = command & 0x3F;
            if count == 0 {
                break;
            }
            out.extend_from_slice(src.get(at..at + count)?);
            at += count;
        } else if command == 0xFE {
            let count = word(&mut at)?;
            let value = byte(&mut at)? as u8;
            out.extend(std::iter::repeat_n(value, count));
        } else {
            // Copy from an absolute output position.
            let count = if command == 0xFF {
                word(&mut at)?
            } else {
                (command & 0x3F) + 3
            };
            let from = word(&mut at)?;
            copy(&mut out, from, count)?;
        }
        if out.len() > max_len {
            return None;
        }
    }
    Some(out)
}
