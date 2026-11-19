//! Generated tray icon, so the tray works without shipping an image asset.

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
}
