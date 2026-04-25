use std::path::{Path, PathBuf};

pub fn save_edit_pixels(
    path: &Path,
    ext: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<PathBuf, String> {
    let output_path = if matches!(ext, "jpg" | "jpeg" | "png") {
        path.to_path_buf()
    } else {
        path.with_extension("png")
    };
    let output_ext = output_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let temp_path = unique_temp_path(&output_path)?;

    let write_result = match output_ext.as_str() {
        "jpg" | "jpeg" => {
            let rgb: Vec<u8> = rgba
                .chunks_exact(4)
                .flat_map(|px| [px[0], px[1], px[2]])
                .collect();
            image::save_buffer_with_format(
                &temp_path,
                &rgb,
                width,
                height,
                image::ColorType::Rgb8,
                image::ImageFormat::Jpeg,
            )
        }
        "png" => image::save_buffer_with_format(
            &temp_path,
            rgba,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        ),
        _ => unreachable!("output extension is normalized to jpg/jpeg/png"),
    };

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err.to_string());
    }

    if matches!(output_ext.as_str(), "jpg" | "jpeg") {
        if let Err(err) = copy_metadata_for_baked_pixels(path, &temp_path, width, height) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }
    }

    if output_path == path {
        if let Ok(metadata) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&temp_path, metadata.permissions());
        }
    }

    if let Err(err) = std::fs::rename(&temp_path, &output_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err.to_string());
    }

    Ok(output_path)
}

pub fn requires_jpeg_reencode_warning(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| ext.to_ascii_lowercase()),
        Some(ext) if matches!(ext.as_str(), "jpg" | "jpeg")
    )
}

fn copy_metadata_for_baked_pixels(
    source_path: &Path,
    output_path: &Path,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let _guard = crate::metadata::exif::rexiv2_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let Ok(metadata) = rexiv2::Metadata::new_from_path(source_path) else {
        return Ok(());
    };

    metadata.set_orientation(rexiv2::Orientation::Normal);
    let _ = metadata.set_tag_string("Exif.Photo.PixelXDimension", &width.to_string());
    let _ = metadata.set_tag_string("Exif.Photo.PixelYDimension", &height.to_string());
    let _ = metadata.set_tag_string("Exif.Image.ImageWidth", &width.to_string());
    let _ = metadata.set_tag_string("Exif.Image.ImageLength", &height.to_string());

    metadata
        .save_to_file(output_path)
        .map_err(|err| err.to_string())
}

fn unique_temp_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let pid = std::process::id();

    for attempt in 0..1000 {
        let temp_path = parent.join(format!(".{filename}.sharpr-save-{pid}-{attempt}.tmp"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(_) => return Ok(temp_path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.to_string()),
        }
    }

    Err("could not create a unique temporary save path".to_string())
}
