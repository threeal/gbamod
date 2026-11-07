//! Captured frames of the GBA screen.

/// The width of the GBA screen in pixels.
pub const WIDTH: usize = 240;

/// The height of the GBA screen in pixels.
pub const HEIGHT: usize = 160;

/// The reason a PNG image can't be read as a frame of the screen.
#[derive(Debug, thiserror::Error)]
pub enum ScreenError {
    /// The data isn't a valid PNG image.
    #[error("not a valid PNG image: {0}")]
    Decode(#[from] png::DecodingError),
    /// The image isn't the size of the screen.
    #[error("the image is {width}x{height}, but the screen is 240x160")]
    Size {
        /// The image's width in pixels.
        width: u32,
        /// The image's height in pixels.
        height: u32,
    },
    /// The image has no color channels.
    #[error("the image is grayscale, but a screenshot is in color")]
    Grayscale,
}

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

    /// Decodes a frame from a 240x160 PNG image in color, such as one saved
    /// by [`Screen::encode_png`], dropping any alpha channel.
    ///
    /// # Errors
    ///
    /// Returns a [`ScreenError`] if the data isn't a valid PNG image, or the
    /// image isn't 240x160 or is grayscale.
    pub fn decode_png(png: &[u8]) -> Result<Self, ScreenError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info()?;
        let (width, height) = (reader.info().width, reader.info().height);
        if (width as usize, height as usize) != (WIDTH, HEIGHT) {
            return Err(ScreenError::Size { width, height });
        }
        let channels = match reader.output_color_type().0 {
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            _ => return Err(ScreenError::Grayscale),
        };
        let mut buffer = vec![0; reader.output_buffer_size().unwrap_or_default()];
        reader.next_frame(&mut buffer)?;
        let rgb = buffer
            .chunks_exact(channels)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();
        Ok(Self { rgb })
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

    /// Encodes `data` as a PNG image of the given size and color type.
    fn png_image(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(data)
            .unwrap();
        png
    }

    #[test]
    fn decode_encoded_png() {
        let screen = Screen::from_frame_buffer(&frame_buffer(256), 256);
        assert_eq!(Screen::decode_png(&screen.encode_png()).unwrap(), screen);
    }

    #[test]
    fn decode_png_with_alpha() {
        let rgba: Vec<u8> = [1, 2, 3, 255].repeat(WIDTH * HEIGHT);
        let png = png_image(240, 160, png::ColorType::Rgba, &rgba);
        let screen = Screen::decode_png(&png).unwrap();
        assert_eq!(screen.pixel(0, 0), [1, 2, 3]);
        assert_eq!(screen.pixel(239, 159), [1, 2, 3]);
    }

    #[test]
    fn reject_invalid_png() {
        let err = Screen::decode_png(b"not a png").unwrap_err();
        assert!(matches!(err, ScreenError::Decode(_)));
        assert!(err.to_string().starts_with("not a valid PNG image: "));
    }

    #[test]
    fn reject_png_of_wrong_size() {
        let png = png_image(2, 1, png::ColorType::Rgb, &[0; 6]);
        assert_eq!(
            Screen::decode_png(&png).unwrap_err().to_string(),
            "the image is 2x1, but the screen is 240x160"
        );
    }

    #[test]
    fn reject_grayscale_png() {
        let png = png_image(
            240,
            160,
            png::ColorType::Grayscale,
            &vec![0; WIDTH * HEIGHT],
        );
        assert_eq!(
            Screen::decode_png(&png).unwrap_err().to_string(),
            "the image is grayscale, but a screenshot is in color"
        );
    }

    #[test]
    fn reject_truncated_png() {
        let screen = Screen::from_frame_buffer(&frame_buffer(256), 256);
        let png = screen.encode_png();
        let err = Screen::decode_png(&png[..png.len() - 20]).unwrap_err();
        assert!(matches!(err, ScreenError::Decode(_)));
    }
}
