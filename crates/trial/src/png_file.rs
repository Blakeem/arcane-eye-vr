//! The PNG writer: an RGB image to a PNG file through the `png` crate.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use png::{BitDepth, ColorType, Encoder, EncodingError};

use crate::pixels::{Depth, RgbImage};

/// Why a PNG file was not written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PngError {
    #[error("could not create {}", path.display())]
    Create { path: PathBuf, source: io::Error },
    #[error("could not write the PNG file {}", path.display())]
    Encode { path: PathBuf, source: EncodingError },
}

/// Writes `image` to a new PNG file at `path`, replacing a file that is there.
pub(crate) fn write(path: &Path, image: &RgbImage) -> Result<(), PngError> {
    let file = match File::create(path) {
        Ok(file) => file,
        Err(source) => return Err(PngError::Create { path: path.to_path_buf(), source }),
    };
    match encode(BufWriter::new(file), image) {
        Ok(()) => Ok(()),
        Err(source) => Err(PngError::Encode { path: path.to_path_buf(), source }),
    }
}

fn encode(out: BufWriter<File>, image: &RgbImage) -> Result<(), EncodingError> {
    let mut encoder = Encoder::new(out, image.width, image.height);
    encoder.set_color(ColorType::Rgb);
    encoder.set_depth(match image.depth {
        Depth::Eight => BitDepth::Eight,
        Depth::Sixteen => BitDepth::Sixteen,
    });
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.data)?;
    writer.finish()
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::BufReader;

    use png::{BitDepth, ColorType, Decoder};

    use super::{PngError, write};
    use crate::pixels::{Depth, RgbImage};
    use crate::test_dir;

    /// The size, bit depth, color type and bytes of the PNG file at `path`.
    fn read_back(path: &std::path::Path) -> (u32, u32, BitDepth, ColorType, Vec<u8>) {
        let file = BufReader::new(File::open(path).expect("open the PNG file"));
        let mut reader = Decoder::new(file).read_info().expect("a PNG header");
        let mut data = vec![0; reader.output_buffer_size().expect("a buffer size")];
        let info = reader.next_frame(&mut data).expect("the image data");
        data.truncate(info.buffer_size());
        (info.width, info.height, info.bit_depth, info.color_type, data)
    }

    #[test]
    fn an_eight_bit_image_reads_back_with_its_size_depth_and_bytes() {
        let dir = test_dir("png-8");
        let path = dir.path().join("frame-0001.png");
        let data = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        write(&path, &RgbImage { width: 2, height: 2, depth: Depth::Eight, data: data.clone() })
            .expect("write the PNG file");
        assert_eq!(read_back(&path), (2, 2, BitDepth::Eight, ColorType::Rgb, data));
    }

    #[test]
    fn a_sixteen_bit_image_reads_back_with_its_size_depth_and_bytes() {
        let dir = test_dir("png-16");
        let path = dir.path().join("frame-0002.png");
        let data = vec![0xFF, 0xFF, 0x80, 0x20, 0x00, 0x40, 0x00, 0x00, 0x00, 0x40, 0xFF, 0xFF];
        write(&path, &RgbImage { width: 1, height: 2, depth: Depth::Sixteen, data: data.clone() })
            .expect("write the PNG file");
        assert_eq!(read_back(&path), (1, 2, BitDepth::Sixteen, ColorType::Rgb, data));
    }

    #[test]
    fn a_file_in_a_missing_folder_is_a_create_error_that_names_it() {
        let dir = test_dir("png-missing");
        let path = dir.path().join(r"missing\frame-0001.png");
        let image = RgbImage { width: 1, height: 1, depth: Depth::Eight, data: vec![1, 2, 3] };
        let error = write(&path, &image).expect_err("no folder");
        assert!(matches!(&error, PngError::Create { path: failed, .. } if *failed == path));
        assert_eq!(error.to_string(), format!("could not create {}", path.display()));
    }

    #[test]
    fn data_shorter_than_the_image_is_an_encode_error_that_names_the_file() {
        let dir = test_dir("png-short");
        let path = dir.path().join("frame-0001.png");
        let image = RgbImage { width: 2, height: 1, depth: Depth::Eight, data: vec![1, 2, 3] };
        let error = write(&path, &image).expect_err("3 bytes for 2 pixels");
        assert!(matches!(&error, PngError::Encode { path: failed, .. } if *failed == path));
    }
}
