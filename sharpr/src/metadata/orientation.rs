use std::path::Path;

use image::DynamicImage;

pub fn apply_exif_orientation_value(img: DynamicImage, orientation: u32) -> DynamicImage {
    use image::imageops;

    match orientation {
        1 => img,
        2 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&img.into_rgba8())),
        3 => DynamicImage::ImageRgba8(imageops::rotate180(&img.into_rgba8())),
        4 => DynamicImage::ImageRgba8(imageops::flip_vertical(&img.into_rgba8())),
        5 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&imageops::rotate90(
            &img.into_rgba8(),
        ))),
        6 => DynamicImage::ImageRgba8(imageops::rotate90(&img.into_rgba8())),
        7 => DynamicImage::ImageRgba8(imageops::flip_horizontal(&imageops::rotate270(
            &img.into_rgba8(),
        ))),
        8 => DynamicImage::ImageRgba8(imageops::rotate270(&img.into_rgba8())),
        _ => img,
    }
}

pub fn apply_exif_orientation(img: DynamicImage, path: &Path) -> DynamicImage {
    apply_exif_orientation_value(img, exif_orientation(path))
}

pub fn exif_orientation(path: &Path) -> u32 {
    extract_exif_data(path, 0).1
}

pub fn extract_exif_data(path: &Path, min_preview_height: u32) -> (Option<DynamicImage>, u32) {
    let mut orientation = 1;
    let Some((little_endian, tiff_vec)) = read_exif_tiff(path) else {
        return (None, orientation);
    };
    let tiff = tiff_vec.as_slice();

    let read_u16 = |buf: &[u8], offset: usize| -> Option<u16> {
        let b = buf.get(offset..offset + 2)?;
        Some(if little_endian {
            u16::from_le_bytes(b.try_into().ok()?)
        } else {
            u16::from_be_bytes(b.try_into().ok()?)
        })
    };
    let read_u32 = |buf: &[u8], offset: usize| -> Option<u32> {
        let b = buf.get(offset..offset + 4)?;
        Some(if little_endian {
            u32::from_le_bytes(b.try_into().ok()?)
        } else {
            u32::from_be_bytes(b.try_into().ok()?)
        })
    };

    let ifd0_offset = read_u32(tiff, 4).unwrap_or(0) as usize;
    if ifd0_offset == 0 {
        return (None, orientation);
    }
    let entry_count = read_u16(tiff, ifd0_offset).unwrap_or(0) as usize;

    for i in 0..entry_count {
        if let Some(entry_offset) = ifd0_offset
            .checked_add(2)
            .and_then(|x| x.checked_add(i.checked_mul(12)?))
        {
            if let Some(tag) = read_u16(tiff, entry_offset) {
                if tag == 0x0112 {
                    if let Some(val) = read_u16(tiff, entry_offset + 8) {
                        orientation = val as u32;
                    }
                }
            }
        }
    }

    let next_ifd_ptr = ifd0_offset
        .checked_add(2)
        .and_then(|x| x.checked_add(entry_count.checked_mul(12).unwrap_or(0)));

    let mut thumb_img = None;
    if let Some(ptr) = next_ifd_ptr {
        if let Some(ifd1_offset) = read_u32(tiff, ptr) {
            let ifd1_offset = ifd1_offset as usize;
            if ifd1_offset > 0 && ifd1_offset < tiff.len() {
                if let Some(ifd1_entries) = read_u16(tiff, ifd1_offset) {
                    let ifd1_entries = ifd1_entries as usize;
                    let mut thumb_offset = None;
                    let mut thumb_len = None;
                    for i in 0..ifd1_entries {
                        if let Some(entry_offset) = ifd1_offset
                            .checked_add(2)
                            .and_then(|x| x.checked_add(i.checked_mul(12).unwrap_or(0)))
                        {
                            if let Some(tag) = read_u16(tiff, entry_offset) {
                                match tag {
                                    0x0201 => {
                                        thumb_offset =
                                            Some(read_u32(tiff, entry_offset + 8).unwrap_or(0)
                                                as usize)
                                    }
                                    0x0202 => {
                                        thumb_len =
                                            Some(read_u32(tiff, entry_offset + 8).unwrap_or(0)
                                                as usize)
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    if let (Some(o), Some(l)) = (thumb_offset, thumb_len) {
                        if let Some(end) = o.checked_add(l) {
                            if let Some(thumb_bytes) = tiff.get(o..end) {
                                if let Ok(img) = image::load_from_memory(thumb_bytes) {
                                    if img.height() >= min_preview_height {
                                        thumb_img = Some(img);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    (thumb_img, orientation)
}

fn read_exif_tiff(path: &Path) -> Option<(bool, Vec<u8>)> {
    use std::io::{BufReader, Read, Seek, SeekFrom};

    let file = std::fs::File::open(path).ok()?;
    let mut r = BufReader::new(file);

    let mut soi = [0u8; 2];
    r.read_exact(&mut soi).ok()?;
    if soi != [0xFF, 0xD8] {
        return None;
    }

    loop {
        let mut marker = [0u8; 2];
        r.read_exact(&mut marker).ok()?;
        if marker[0] != 0xFF {
            return None;
        }

        let marker_type = marker[1];
        if marker_type == 0xE1 {
            let mut len_buf = [0u8; 2];
            r.read_exact(&mut len_buf).ok()?;
            let segment_len = u16::from_be_bytes(len_buf) as usize;
            if segment_len < 2 {
                return None;
            }
            let data_len = segment_len - 2;
            let mut data = vec![0u8; data_len];
            r.read_exact(&mut data).ok()?;

            if data.len() < 6 || &data[..6] != b"Exif\0\0" {
                continue;
            }
            let tiff = data[6..].to_vec();
            if tiff.len() < 8 {
                return None;
            }

            let little_endian = match &tiff[..2] {
                b"II" => true,
                b"MM" => false,
                _ => return None,
            };
            return Some((little_endian, tiff));
        } else if marker_type == 0xDA || marker_type == 0xD9 {
            return None;
        } else {
            let mut len_buf = [0u8; 2];
            r.read_exact(&mut len_buf).ok()?;
            let segment_len = u16::from_be_bytes(len_buf) as usize;
            if segment_len < 2 {
                return None;
            }
            r.seek(SeekFrom::Current((segment_len - 2) as i64)).ok()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sharpr-{name}-{}-{nanos}.jpg", std::process::id()))
    }

    fn pixel_strip() -> DynamicImage {
        let mut img = RgbaImage::new(2, 1);
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([0, 0, 255, 255]));
        DynamicImage::ImageRgba8(img)
    }

    fn jpeg_with_orientation(orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());

        let mut exif = Vec::new();
        exif.extend_from_slice(b"Exif\0\0");
        exif.extend_from_slice(&tiff);

        let mut jpeg = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]);
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        jpeg.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    #[test]
    fn apply_exif_orientation_rotates_clockwise() {
        let out = apply_exif_orientation_value(pixel_strip(), 6).to_rgba8();
        assert_eq!(out.dimensions(), (1, 2));
        assert_eq!(*out.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
        assert_eq!(*out.get_pixel(0, 1), Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn apply_exif_orientation_flips_horizontally() {
        let out = apply_exif_orientation_value(pixel_strip(), 2).to_rgba8();
        assert_eq!(out.dimensions(), (2, 1));
        assert_eq!(*out.get_pixel(0, 0), Rgba([0, 0, 255, 255]));
        assert_eq!(*out.get_pixel(1, 0), Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn extract_exif_data_reads_orientation_from_app1_segment() {
        let path = temp_path("orientation");
        std::fs::write(&path, jpeg_with_orientation(8)).unwrap();
        let (_thumb, orientation) = extract_exif_data(&path, 0);
        let _ = std::fs::remove_file(&path);
        assert_eq!(orientation, 8);
    }

    #[test]
    fn extract_exif_data_defaults_on_non_jpeg() {
        let path = temp_path("not-jpeg");
        std::fs::write(&path, b"not a jpeg").unwrap();
        let (thumb, orientation) = extract_exif_data(&path, 0);
        let _ = std::fs::remove_file(&path);
        assert!(thumb.is_none());
        assert_eq!(orientation, 1);
    }
}
