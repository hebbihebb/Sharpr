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
