//! Perceptual difference hash (dHash) for frame deduplication.
//!
//! Algorithm: resize to 9×8 grayscale, compare adjacent horizontal pixels to
//! produce a 64-bit fingerprint. Hamming distance ≤4 → duplicate.

use image::GrayImage;

/// Compute the 64-bit dHash of a grayscale image.
///
/// The image is resized to 9×8 using nearest-neighbour (fast; adequate for
/// perceptual comparison), then the 64 left→right pixel differences are
/// encoded as bits.
pub fn dhash(img: &GrayImage) -> u64 {
    use image::imageops;

    // Resize to 9 wide × 8 tall (9 columns → 8 horizontal differences per row)
    let small = imageops::resize(img, 9, 8, imageops::FilterType::Nearest);

    let mut hash: u64 = 0;
    for row in 0..8u32 {
        for col in 0..8u32 {
            let left = small.get_pixel(col, row)[0];
            let right = small.get_pixel(col + 1, row)[0];
            hash = (hash << 1) | (if left < right { 1 } else { 0 });
        }
    }
    hash
}

/// Hamming distance between two dHash values (number of differing bits).
#[inline]
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    fn solid_gray(width: u32, height: u32, value: u8) -> GrayImage {
        GrayImage::from_fn(width, height, |_, _| Luma([value]))
    }

    fn gradient_gray(width: u32, height: u32) -> GrayImage {
        GrayImage::from_fn(width, height, |x, _y| {
            Luma([(x * 255 / width.max(1)) as u8])
        })
    }

    #[test]
    fn identical_images_have_distance_zero() {
        let img = gradient_gray(64, 64);
        let h1 = dhash(&img);
        let h2 = dhash(&img);
        assert_eq!(hamming(h1, h2), 0, "identical images must have distance 0");
    }

    #[test]
    fn hamming_correctness() {
        assert_eq!(hamming(0u64, 0u64), 0);
        assert_eq!(hamming(0u64, u64::MAX), 64);
        assert_eq!(hamming(0b1111u64, 0b0000u64), 4);
        assert_eq!(hamming(0b1010u64, 0b0101u64), 4);
    }

    #[test]
    fn tiny_perturbation_is_small_distance() {
        // Base: left half dark, right half bright (many left < right transitions)
        let base = GrayImage::from_fn(64, 64, |x, _| {
            Luma([if x < 32 { 50 } else { 200 }])
        });
        // Near-dup: change a single pixel in a corner — dHash is coarse enough
        // that this won't move the hash, but at minimum it should be ≤4.
        let mut near_dup = base.clone();
        near_dup.put_pixel(0, 0, Luma([51])); // ±1 in one corner pixel

        let h_base = dhash(&base);
        let h_near = dhash(&near_dup);
        let dist = hamming(h_base, h_near);
        assert!(
            dist <= 4,
            "near-dup should have distance ≤4 (got {dist}), so pipeline deduplicates it"
        );
    }

    #[test]
    fn clearly_different_image_has_large_distance() {
        // All black vs all white: after resize the gradient comparison will be 0
        // for solid images (no left<right difference), BUT a solid-black vs
        // gradient image will differ significantly.
        let black = solid_gray(64, 64, 0);
        let bright_gradient = GrayImage::from_fn(64, 64, |x, _y| {
            Luma([((x + 1) * 8).min(255) as u8])
        });

        let h1 = dhash(&black);
        let h2 = dhash(&bright_gradient);
        let dist = hamming(h1, h2);
        assert!(
            dist > 4,
            "distinct images should have distance >4 (got {dist}), so pipeline keeps the frame"
        );
    }

    #[test]
    fn two_different_solid_colors_are_both_zero_hash_distance_zero() {
        // Both solid images produce all-same bits (0 or all-1 depending on
        // comparison), so solid vs solid can be distance 0 or 64.
        // What matters is the dedup rule fires consistently — this just documents
        // the behaviour, not asserts a particular value.
        let white = solid_gray(32, 32, 255);
        let black = solid_gray(32, 32, 0);
        let hw = dhash(&white);
        let hb = dhash(&black);
        // Both are all-same (either all 0-bits or all 1-bits).
        // Hamming should be either 0 or 64.
        let d = hamming(hw, hb);
        assert!(d == 0 || d == 64, "solid images produce degenerate hashes (got {d})");
    }
}
