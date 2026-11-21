// SPDX-License-Identifier: LGPL-3.0-or-later
//! Generated application icon (tray and window), so no image asset has to be shipped.

/// ARGB32, network byte order (A, R, G, B per pixel), as the StatusNotifierItem
/// `IconPixmap` property expects: a filled anti-aliased circle on a transparent background.
pub fn circle_argb(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let c = size as f32 / 2.0;
    let radius = c - 1.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let dist = (dx * dx + dy * dy).sqrt();
            // one pixel of edge falloff
            let alpha = (radius + 0.5 - dist).clamp(0.0, 1.0);
            out.extend_from_slice(&[(alpha * 255.0).round() as u8, rgb[0], rgb[1], rgb[2]]);
        }
    }
    out
}

/// PNG (RGBA, 8 bit) of the same circle. Uses stored (uncompressed) deflate blocks, so no
/// compression crate is needed; at these sizes the file is a few tens of KB at most.
pub fn circle_png(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let argb = circle_argb(size, rgb);
    let mut raw = Vec::with_capacity((size * (size * 4 + 1)) as usize);
    for row in argb.chunks((size * 4) as usize) {
        raw.push(0); // filter type: none
        for px in row.chunks(4) {
            raw.extend_from_slice(&[px[1], px[2], px[3], px[0]]); // ARGB -> RGBA
        }
    }

    let mut z = vec![0x78, 0x01]; // zlib header: deflate, 32K window, no compression
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(block) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none())); // BFINAL, BTYPE=00 (stored)
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&size.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bit, RGBA, deflate, no filter, no interlace

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    push_chunk(&mut png, b"IHDR", &ihdr);
    push_chunk(&mut png, b"IDAT", &z);
    push_chunk(&mut png, b"IEND", &[]);
    png
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_shape() {
        let d = circle_argb(32, [0x58, 0x65, 0xf2]);
        assert_eq!(d.len(), 32 * 32 * 4);
        let px = |x: usize, y: usize| &d[(y * 32 + x) * 4..(y * 32 + x) * 4 + 4];
        assert_eq!(px(16, 16), &[255, 0x58, 0x65, 0xf2], "centre is opaque");
        assert_eq!(px(0, 0)[0], 0, "corner is transparent");
        assert_eq!(px(31, 31)[0], 0);
    }

    /// Walk the PNG, check every chunk CRC, un-wrap the stored deflate blocks and compare
    /// the pixels with the ARGB source.
    #[test]
    fn png_is_well_formed_and_matches_pixels() {
        let size = 64u32;
        let png = circle_png(size, [0x58, 0x65, 0xf2]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let mut pos = 8;
        let mut idat = Vec::new();
        let mut kinds = Vec::new();
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &png[pos + 4..pos + 8];
            let data = &png[pos + 8..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(
                crc,
                crc32(&png[pos + 4..pos + 8 + len]),
                "bad CRC in {kind:?}"
            );
            kinds.push(kind.to_vec());
            if kind == b"IHDR" {
                assert_eq!(u32::from_be_bytes(data[0..4].try_into().unwrap()), size);
                assert_eq!(u32::from_be_bytes(data[4..8].try_into().unwrap()), size);
            }
            if kind == b"IDAT" {
                idat.extend_from_slice(data);
            }
            pos += 12 + len;
        }
        assert_eq!(
            kinds,
            [b"IHDR".to_vec(), b"IDAT".to_vec(), b"IEND".to_vec()]
        );

        // zlib header, stored blocks, adler trailer
        assert_eq!(&idat[..2], &[0x78, 0x01]);
        let mut p = 2;
        let mut raw = Vec::new();
        loop {
            let last = idat[p] & 1 == 1;
            let len = u16::from_le_bytes([idat[p + 1], idat[p + 2]]) as usize;
            let nlen = u16::from_le_bytes([idat[p + 3], idat[p + 4]]);
            assert_eq!(nlen, !(len as u16));
            raw.extend_from_slice(&idat[p + 5..p + 5 + len]);
            p += 5 + len;
            if last {
                break;
            }
        }
        assert_eq!(
            u32::from_be_bytes(idat[p..p + 4].try_into().unwrap()),
            adler32(&raw)
        );

        let argb = circle_argb(size, [0x58, 0x65, 0xf2]);
        assert_eq!(raw.len(), (size * (size * 4 + 1)) as usize);
        for y in 0..size as usize {
            let row = &raw[y * (size as usize * 4 + 1)..(y + 1) * (size as usize * 4 + 1)];
            assert_eq!(row[0], 0, "filter byte");
            for x in 0..size as usize {
                let a = &argb[(y * size as usize + x) * 4..][..4];
                assert_eq!(&row[1 + x * 4..1 + x * 4 + 4], &[a[1], a[2], a[3], a[0]]);
            }
        }
    }

    #[test]
    fn large_png_spans_multiple_stored_blocks() {
        // 256x256 RGBA is 262_400 raw bytes, more than one 65_535-byte block.
        let png = circle_png(256, [1, 2, 3]);
        assert!(png.len() > 262_400);
    }
}
