//! What the output shows where no surface is: one color, the brand's wash unless the operator chose another, with the
//! easi.run logo in its middle at a fifth of the output's width (at most the logo's own width). It is drawn once, at
//! start, for the output's one size, and compositions copy from it what they uncover (see [`crate::compose`]).

/// The mark and the name as the landing draws them, straight RGBA on a transparent ground, made by display/logo.mjs.
const LOGO: &[u8] = include_bytes!("logo.qoi");

/// The brand's wash (`--color-wash` in client/src/app.css).
pub const WASH: u32 = 0xf3f6fb;

/// The background of a `width` × `height` output, XRGB8888 with full alpha (as compositions write it), over `color`.
pub fn draw(width: i32, height: i32, color: u32) -> Vec<u32> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = vec![0xff00_0000 | color; width * height];
    let (header, rgba) = qoi::decode_to_vec(LOGO).expect("the built-in logo decodes");
    let (sw, sh) = (header.width as usize, header.height as usize);
    // A fifth of the width, at most the logo's own, its height in proportion (at least a pixel each).
    let dw = (width / 5).clamp(1, sw);
    let dh = (sh * dw / sw).max(1);
    let logo = shrink(&rgba, sw, sh, dw, dh);
    let (x0, y0) = ((width - dw) / 2, (height.saturating_sub(dh)) / 2);
    for (y, row) in logo.chunks(dw).take(height).enumerate() {
        let out = &mut pixels[(y0 + y) * width + x0..][..dw];
        for (d, [r, g, b, a]) in out.iter_mut().zip(row) {
            let over = |p: f32, shift: u32| {
                let under = ((color >> shift) & 0xff) as f32;
                ((p + under * (1.0 - a / 255.0)).round().clamp(0.0, 255.0) as u32) << shift
            };
            *d = 0xff00_0000 | over(*r, 16) | over(*g, 8) | over(*b, 0);
        }
    }
    pixels
}

/// `rgba` (straight, `sw` × `sh`) resampled to `dw` × `dh`, no larger, by the share of each source pixel each output
/// pixel covers: premultiplied RGBA, 0-255 a channel.
fn shrink(rgba: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<[f32; 4]> {
    let premultiplied: Vec<[f32; 4]> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, a]| {
            let share = f32::from(a) / 255.0;
            [f32::from(r) * share, f32::from(g) * share, f32::from(b) * share, f32::from(a)]
        })
        .collect();
    // Across, then down: each output pixel's source pixels with the share of it they cover.
    let across = shares(sw, dw);
    let mut rows = vec![[0f32; 4]; sh * dw];
    for y in 0..sh {
        for (x, share) in across.iter().enumerate() {
            rows[y * dw + x] = sum(share.iter().map(|&(i, s)| (premultiplied[y * sw + i], s)));
        }
    }
    let down = shares(sh, dh);
    let mut out = vec![[0f32; 4]; dh * dw];
    for (y, share) in down.iter().enumerate() {
        for x in 0..dw {
            out[y * dw + x] = sum(share.iter().map(|&(i, s)| (rows[i * dw + x], s)));
        }
    }
    out
}

/// For each of `to` output pixels along a side of `from` source pixels, the source pixels it covers and the share of
/// it each one takes (summing to 1).
fn shares(from: usize, to: usize) -> Vec<Vec<(usize, f32)>> {
    let step = from as f64 / to as f64;
    (0..to)
        .map(|i| {
            let (start, end) = (i as f64 * step, (i + 1) as f64 * step);
            (start.floor() as usize..(end.ceil() as usize).min(from))
                .map(|j| (j, ((end.min(j as f64 + 1.0) - start.max(j as f64)) / step) as f32))
                .collect()
        })
        .collect()
}

fn sum(parts: impl Iterator<Item = ([f32; 4], f32)>) -> [f32; 4] {
    parts.fold([0.0; 4], |acc, (p, s)| [acc[0] + p[0] * s, acc[1] + p[1] * s, acc[2] + p[2] * s, acc[3] + p[3] * s])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bounds of the pixels that are not the plain color: `(x, y, w, h)`.
    fn drawn(pixels: &[u32], width: usize, color: u32) -> (usize, usize, usize, usize) {
        let marked: Vec<(usize, usize)> =
            (0..pixels.len()).filter(|&i| pixels[i] != 0xff00_0000 | color).map(|i| (i % width, i / width)).collect();
        let (x0, x1) = (marked.iter().map(|p| p.0).min().unwrap(), marked.iter().map(|p| p.0).max().unwrap());
        let (y0, y1) = (marked.iter().map(|p| p.1).min().unwrap(), marked.iter().map(|p| p.1).max().unwrap());
        (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    }

    #[test]
    fn the_logo_sits_in_the_middle_at_a_fifth_of_the_width_at_most_its_own() {
        let (header, _) = qoi::decode_to_vec(LOGO).unwrap();
        let own = header.width as usize;
        for (width, height) in [(320, 240), (1280, 800), (1920, 1080), (641, 479), (333, 101)] {
            let pixels = draw(width, height, WASH);
            let (width, height) = (width as usize, height as usize);
            assert_eq!(pixels.len(), width * height);
            for corner in [0, width - 1, width * (height - 1), width * height - 1] {
                assert_eq!(pixels[corner], 0xff00_0000 | WASH, "{width}x{height}: the color around the logo");
            }
            let (x, y, w, h) = drawn(&pixels, width, WASH);
            let wanted = (width / 5).min(own);
            assert!(w <= wanted && w + 2 >= wanted, "{width}x{height}: the logo is {w} wide, not about {wanted}");
            assert!(x.abs_diff(width - x - w) <= 2, "{width}x{height}: centered across, from {x}");
            assert!(y.abs_diff(height - y - h) <= 2, "{width}x{height}: centered down, from {y}");
        }
    }

    #[test]
    fn the_logo_is_drawn_over_the_color_chosen() {
        let (width, height) = (1280, 800);
        let dark = draw(width, height, 0x000000);
        let wash = draw(width, height, WASH);
        assert!(dark.iter().all(|p| p >> 24 == 0xff), "every pixel has full alpha");
        assert_eq!(dark[0], 0xff00_0000);
        // The mark's opaque blue is the same over either ground; its edges are not.
        let blue: Vec<usize> = (0..dark.len()).filter(|&i| dark[i] == 0xff24_57f5).collect();
        assert!(blue.len() > 100 && blue.iter().all(|&i| wash[i] == dark[i]), "the mark's blue, whatever the ground");
        assert_ne!(drawn(&dark, width as usize, 0), (0, 0, width as usize, height as usize));
        assert!((0..dark.len()).any(|i| dark[i] != wash[i] && dark[i] != 0xff00_0000), "edges blend with the ground");
    }

    #[test]
    fn a_shrink_keeps_the_average() {
        // White beside transparent, into one pixel: half white, premultiplied, and half covered.
        assert_eq!(shrink(&[255, 255, 255, 255, 0, 0, 0, 0], 2, 1, 1, 1), vec![[127.5; 4]]);
        // Three pixels into two: each takes one whole and half of the middle, which is a third of it.
        let shares = shares(3, 2);
        assert_eq!(
            shares.iter().map(|s| s.iter().map(|p| p.0).collect::<Vec<_>>()).collect::<Vec<_>>(),
            [[0, 1], [1, 2]]
        );
        for (share, wanted) in shares.iter().flatten().zip([2.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0]) {
            assert!((share.1 - wanted).abs() < 1e-6, "{share:?}");
        }
    }
}
