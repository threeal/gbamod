//! Captured frames of the GBA screen.

/// The width of the GBA screen in pixels.
pub const WIDTH: usize = 240;

/// The height of the GBA screen in pixels.
pub const HEIGHT: usize = 160;

/// One frame of the GBA screen, as 8-bit RGB pixels in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    rgb: Vec<u8>,
}

impl Screen {
    /// Copies a frame out of a buffer of 32-bit pixels that hold red, green,
    /// and blue in their low three bytes, with `stride` pixels per row.
    ///
    /// # Panics
    ///
    /// Panics if `stride` is narrower than the screen, or if the buffer holds
    /// fewer than [`HEIGHT`] rows.
    #[must_use]
    pub fn from_frame_buffer(buffer: &[u32], stride: usize) -> Self {
        assert!(
            stride >= WIDTH,
            "stride {stride} is narrower than the screen"
        );
        assert!(
            buffer.len() >= stride * (HEIGHT - 1) + WIDTH,
            "frame buffer of {} pixels is too small",
            buffer.len()
        );
        let rgb = buffer
            .chunks(stride)
            .take(HEIGHT)
            .flat_map(|row| &row[..WIDTH])
            .flat_map(|pixel| {
                let [red, green, blue, _] = pixel.to_le_bytes();
                [red, green, blue]
            })
            .collect();
        Self { rgb }
    }

    /// Returns the red, green, and blue values of the pixel at `x` and `y`.
    ///
    /// # Panics
    ///
    /// Panics if the position is outside the screen.
    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        assert!(x < WIDTH && y < HEIGHT, "({x}, {y}) is outside the screen");
        let offset = (y * WIDTH + x) * 3;
        [self.rgb[offset], self.rgb[offset + 1], self.rgb[offset + 2]]
    }

    /// Encodes the frame as an 8-bit RGB PNG image.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the encoder only fails on I/O errors or
    /// mismatched dimensions, and it writes to memory with the screen's own
    /// dimensions.
    #[must_use]
    pub fn encode_png(&self) -> Vec<u8> {
        let mut png = Vec::new();
        #[allow(clippy::cast_possible_truncation)]
        let mut encoder = png::Encoder::new(&mut png, WIDTH as u32, HEIGHT as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&self.rgb))
            .expect("encoding to memory with matching dimensions can't fail");
        png
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a frame buffer whose pixel at (x, y) is red x, green y, blue 7,
    /// with garbage in the unused high byte and in the columns past the screen.
    fn frame_buffer(stride: usize) -> Vec<u32> {
        let mut buffer = vec![0xFFFF_FFFF; stride * HEIGHT];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                buffer[y * stride + x] = u32::from_le_bytes([
                    u8::try_from(x).unwrap(),
                    u8::try_from(y).unwrap(),
                    7,
                    0xAB,
                ]);
            }
        }
        buffer
    }

    #[test]
    fn copy_frame_buffer() {
        let screen = Screen::from_frame_buffer(&frame_buffer(256), 256);
        assert_eq!(screen.pixel(0, 0), [0, 0, 7]);
        assert_eq!(screen.pixel(239, 0), [239, 0, 7]);
        assert_eq!(screen.pixel(12, 159), [12, 159, 7]);
    }

    #[test]
    fn copy_unpadded_frame_buffer() {
        let screen = Screen::from_frame_buffer(&frame_buffer(WIDTH), WIDTH);
        assert_eq!(screen.pixel(239, 159), [239, 159, 7]);
    }

    #[test]
    #[should_panic(expected = "stride 239 is narrower than the screen")]
    fn reject_narrow_stride() {
        let _ = Screen::from_frame_buffer(&frame_buffer(256), 239);
    }

    #[test]
    #[should_panic(expected = "frame buffer of 100 pixels is too small")]
    fn reject_small_frame_buffer() {
        let _ = Screen::from_frame_buffer(&[0; 100], 256);
    }

    #[test]
    #[should_panic(expected = "(240, 0) is outside the screen")]
    fn reject_pixel_outside_screen() {
        let screen = Screen::from_frame_buffer(&frame_buffer(256), 256);
        let _ = screen.pixel(240, 0);
    }

    #[test]
    fn encode_png() {
        let screen = Screen::from_frame_buffer(&frame_buffer(256), 256);
        let png = screen.encode_png();

        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut rgb = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut rgb).unwrap();
        assert_eq!((info.width, info.height), (240, 160));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(rgb, screen.rgb);
    }
}
