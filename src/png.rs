//! The menu row icons, which dbusmenu only takes as PNG bytes: 8-bit RGBA, no
//! filtering, stored (uncompressed) deflate blocks, through the `png` crate.

pub fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(rgba.len(), width as usize * height as usize * 4);
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::NoCompression);
    encoder.set_filter(png::Filter::NoFilter);
    let mut writer = encoder.write_header().expect("a Vec takes any write");
    writer
        .write_image_data(rgba)
        .expect("the pixel count was checked above");
    writer.finish().expect("a Vec takes any write");
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const IEND_WITH_ITS_FIXED_CRC: &[u8] = b"IEND\xAE\x42\x60\x82";

    /// 80,100 bytes of scanlines: more than one 65,535-byte stored block.
    const TWO_STORED_BLOCKS: (u32, u32) = (200, 100);

    #[test]
    fn an_image_is_signature_header_data_end() {
        let png = encode(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..24], &[0, 0, 0, 2, 0, 0, 0, 1]);
        assert_eq!(&png[png.len() - 8..], IEND_WITH_ITS_FIXED_CRC);
    }

    #[test]
    fn rows_over_one_stored_block_split_into_several() {
        let (w, h) = TWO_STORED_BLOCKS;
        let png = encode(w, h, &vec![7; (w * h * 4) as usize]);
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        const ZLIB_HEADER: usize = 2;
        const STORED_BLOCK_HEADER: usize = 5;
        const ADLER32: usize = 4;
        let scanlines = (w as usize * 4 + 1) * h as usize;
        assert_eq!(
            idat_len,
            ZLIB_HEADER + 2 * STORED_BLOCK_HEADER + scanlines + ADLER32
        );
    }

    fn decoded(encoded: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::Cursor::new(encoded))
            .read_info()
            .expect("a PNG header");
        let mut pixels = vec![0; reader.output_buffer_size().expect("a size")];
        let info = reader.next_frame(&mut pixels).expect("PNG image data");
        pixels.truncate(info.buffer_size());
        (info, pixels)
    }

    /// dbusmenu hands these bytes to whatever PNG decoder the tray host
    /// uses, so the contract is what a decoder reads back, not the bytes.
    #[test]
    fn a_standard_decoder_reads_back_the_same_pixels() {
        let (width, height) = TWO_STORED_BLOCKS;
        let pixels: Vec<u8> = (0..width * height * 4)
            .map(|i| (i * 7 % 256) as u8)
            .collect();
        let (info, read_back) = decoded(&encode(width, height, &pixels));
        assert_eq!((info.width, info.height), (width, height));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(read_back, pixels);
    }

    #[test]
    fn a_one_pixel_image_decodes() {
        let (info, read_back) = decoded(&encode(1, 1, &[1, 2, 3, 4]));
        assert_eq!((info.width, info.height), (1, 1));
        assert_eq!(read_back, [1, 2, 3, 4]);
    }
}
