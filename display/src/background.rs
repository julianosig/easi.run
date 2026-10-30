//! What the output shows where no surface is: one color, the brand's wash unless the operator chose another, with a
//! logo in its middle, easi.run's unless the operator gave another (see [`display_protocol::logo`]), at a fifth of the
//! output's width, at most [`MAX_LOGO_WIDTH`] and its own width. It is drawn once, at start, for the output's one size,
//! and compositions copy from it what they uncover (see [`crate::compose`]).

use display_protocol::logo::Logo;

/// The mark and the name as the landing draws them, straight RGBA on a transparent ground, made by display/logo.mjs.
const LOGO: &[u8] = include_bytes!("logo.qoi");

/// The brand's wash (`--color-wash` in client/src/app.css).
pub const WASH: u32 = 0xf3f6fb;

/// The widest a logo is drawn: easi.run's own width, so a large output shows it pixel for pixel.
const MAX_LOGO_WIDTH: usize = 320;

/// easi.run's logo.
pub fn easi_run() -> Logo {
    let (header, rgba) = qoi::decode_to_vec(LOGO).expect("the built-in logo decodes");
    Logo { width: header.width, height: header.height, rgba }
}

/// The background of a `width` × `height` output, XRGB8888 with full alpha (as compositions write it): `logo` over
/// `color`.
pub fn draw(width: i32, height: i32, color: u32, logo: &Logo) -> Vec<u32> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = vec![0xff00_0000 | color; width * height];
    let (sw, sh) = (logo.width as usize, logo.height as usize);
    // A fifth of the width, at most MAX_LOGO_WIDTH and the logo's own, its height in proportion and at most half the
    // output's (a tall logo's), at least a pixel each.
    let mut dw = (width / 5).min(MAX_LOGO_WIDTH).clamp(1, sw);
    let mut dh = (sh * dw / sw).max(1);
    if dh > height / 2 {
        dh = (height / 2).max(1);
        dw = (sw * dh / sh).clamp(1, dw);
    }
    let logo = shrink(&logo.rgba, sw, sh, dw, dh);
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
        let own = easi_run().width as usize;
        assert_eq!(own, MAX_LOGO_WIDTH, "easi.run's logo is drawn at most at its own width");
        for (width, height) in [(320, 240), (1280, 800), (1920, 1080), (641, 479), (333, 101)] {
            let pixels = draw(width, height, WASH, &easi_run());
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
        let dark = draw(width, height, 0x000000, &easi_run());
        let wash = draw(width, height, WASH, &easi_run());
        assert!(dark.iter().all(|p| p >> 24 == 0xff), "every pixel has full alpha");
        assert_eq!(dark[0], 0xff00_0000);
        // The mark's opaque blue is the same over either ground; its edges are not.
        let blue: Vec<usize> = (0..dark.len()).filter(|&i| dark[i] == 0xff24_57f5).collect();
        assert!(blue.len() > 100 && blue.iter().all(|&i| wash[i] == dark[i]), "the mark's blue, whatever the ground");
        assert_ne!(drawn(&dark, width as usize, 0), (0, 0, width as usize, height as usize));
        assert!((0..dark.len()).any(|i| dark[i] != wash[i] && dark[i] != 0xff00_0000), "edges blend with the ground");
    }

    #[test]
    fn an_operator_s_logo_is_drawn_centred_and_scaled_over_the_color() {
        // 640 by 320: its left half opaque red, its right half white at half alpha.
        let rgba: Vec<u8> = (0..640 * 320)
            .flat_map(|i| if i % 640 < 320 { [0xff, 0, 0, 0xff] } else { [0xff, 0xff, 0xff, 0x80] })
            .collect();
        let logo = Logo { width: 640, height: 320, rgba };
        let (width, height) = (1280, 800);
        let pixels = draw(width, height, WASH, &logo);
        // A fifth of 1280 wide, half as high, in the middle.
        assert_eq!(drawn(&pixels, width as usize, WASH), (512, 336, 256, 128));
        let at = |x: usize, y: usize| pixels[y * width as usize + x];
        assert_eq!(at(512 + 60, 400), 0xffff_0000, "the opaque half");
        let half = |under: u32| ((0x80 as f32 + under as f32 * (1.0 - 128.0 / 255.0)).round() as u32).min(255);
        let blended = 0xff00_0000 | half(0xf3) << 16 | half(0xf6) << 8 | half(0xfb);
        assert_eq!(at(512 + 200, 400), blended, "the translucent half, over the wash");
        assert_eq!(at(500, 400), 0xff00_0000 | WASH, "beside it");
        // A tall one is held to half the output's height.
        let tall = Logo { width: 10, height: 1000, rgba: vec![0xff; 10 * 1000 * 4] };
        let (_, _, w, h) = drawn(&draw(width, height, WASH, &tall), width as usize, WASH);
        assert_eq!((w, h), (4, 400));
        // A small one is not enlarged.
        let small = Logo { width: 20, height: 10, rgba: vec![0xff; 20 * 10 * 4] };
        assert_eq!(drawn(&draw(width, height, WASH, &small), width as usize, WASH), (630, 395, 20, 10));
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
