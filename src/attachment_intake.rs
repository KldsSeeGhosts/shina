//! Ported from KSG's chat/composerAttachmentFiles.ts and lib/imageCompression.ts.
use std::io::Cursor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    File,
    UnsupportedImage,
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(base, _)| !base.is_empty())
        .map_or(String::new(), |(_, ext)| ext.to_ascii_lowercase())
}

pub fn native_mime(name: &str) -> &'static str {
    match extension(name).as_str() {
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "svg" => "image/svg+xml",
        "dng" => "image/x-adobe-dng",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "psd" => "image/vnd.adobe.photoshop",
        "cr2" => "image/x-canon-cr2",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

fn generic(mime: &str) -> bool {
    mime.is_empty() || mime.eq_ignore_ascii_case("application/octet-stream")
}

fn generic_media(mime: &str) -> bool {
    matches!(
        mime,
        "" | "application/octet-stream"
            | "binary/octet-stream"
            | "application/unknown"
            | "video/mp2t"
            | "video/vnd.dlna.mpeg-tts"
    )
}

pub fn is_heic(name: &str, mime: &str) -> bool {
    mime.eq_ignore_ascii_case("image/heic")
        || mime.eq_ignore_ascii_case("image/heif")
        || generic(mime) && matches!(extension(name).as_str(), "heic" | "heif")
}

fn is_dng(name: &str, mime: &str) -> bool {
    extension(name) == "dng"
        || matches!(
            mime.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "image/dng" | "image/x-dng" | "image/x-adobe-dng"
        )
}

fn is_svg(name: &str, mime: &str) -> bool {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime == "image/svg+xml" || generic_media(&mime) && extension(name) == "svg"
}

pub fn classify(name: &str, mime: &str) -> Kind {
    if is_svg(name, mime) || is_dng(name, mime) {
        return Kind::File;
    }
    if is_heic(name, mime)
        || generic(mime)
            && matches!(
                extension(name).as_str(),
                "gif" | "jpg" | "jpeg" | "png" | "webp"
            )
    {
        return Kind::Image;
    }
    let mime = mime.to_ascii_lowercase();
    if !mime.starts_with("image/") {
        return Kind::File;
    }
    if matches!(
        mime.as_str(),
        "image/gif" | "image/jpeg" | "image/png" | "image/webp"
    ) {
        return Kind::Image;
    }
    if matches!(
        extension(name).as_str(),
        "avif"
            | "bmp"
            | "gif"
            | "ico"
            | "heic"
            | "heif"
            | "jpeg"
            | "jpg"
            | "png"
            | "svg"
            | "tif"
            | "tiff"
            | "webp"
    ) {
        Kind::File
    } else {
        Kind::UnsupportedImage
    }
}

pub fn file_mime(name: &str, mime: &str) -> String {
    if is_dng(name, mime) {
        "image/x-adobe-dng".into()
    } else if is_svg(name, mime) {
        "image/svg+xml".into()
    } else if generic_media(
        &mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase(),
    ) {
        let inferred = native_mime(name);
        if inferred.starts_with("video/") {
            inferred.into()
        } else {
            "application/octet-stream".into()
        }
    } else {
        mime.into()
    }
}

pub fn claims_paste(name: &str, mime: &str, text: &str) -> bool {
    is_dng(name, mime)
        || is_svg(name, mime)
        || matches!(classify(name, mime), Kind::Image | Kind::UnsupportedImage)
        || text.is_empty()
}

pub const MAX_SOURCE_BYTES: usize = 50 * 1024 * 1024;
const MAX_HEIC_PIXELS: u64 = 64_000_000;
const QUALITY_STEPS: [u8; 4] = [92, 85, 78, 68];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    TooLarge,
    Unreadable,
}

impl Failure {
    pub fn message(self, name: &str) -> String {
        match self {
            Self::TooLarge => format!("'{name}' is too large to attach, even after compression."),
            Self::Unreadable => format!("'{name}' could not be read as an image."),
        }
    }
}

pub fn fit_size(width: u32, height: u32) -> Option<(u32, u32)> {
    let edge = width.max(height);
    if edge <= 2000 {
        return None;
    }
    let scale = 2000.0 / f64::from(edge);
    Some((
        (f64::from(width) * scale).round().clamp(1.0, 2000.0) as u32,
        (f64::from(height) * scale).round().clamp(1.0, 2000.0) as u32,
    ))
}

fn heic_box(data: &[u8], mut offset: usize, end: usize, kind: &[u8; 4]) -> Option<(usize, usize)> {
    while offset.checked_add(8)? <= end {
        let size = u32::from_be_bytes(data.get(offset..offset + 4)?.try_into().ok()?);
        let (size, header) = match size {
            0 => (end - offset, 8),
            1 => (
                usize::try_from(u64::from_be_bytes(
                    data.get(offset + 8..offset + 16)?.try_into().ok()?,
                ))
                .ok()?,
                16,
            ),
            n => (n as usize, 8),
        };
        if size < header || size > end - offset {
            return None;
        }
        if data.get(offset + 4..offset + 8)? == kind {
            return Some((offset + header, offset + size));
        }
        offset += size;
    }
    None
}

pub fn validate_heic_dimensions(bytes: &[u8]) -> Result<(), Failure> {
    let data = &bytes[..bytes.len().min(1024 * 1024)];
    let parse = || {
        let (meta, end) = heic_box(data, 0, data.len(), b"meta")?;
        let (properties, end) = heic_box(data, meta + 4, end, b"iprp")?;
        let (mut offset, end) = heic_box(data, properties, end, b"ipco")?;
        let mut found = false;
        while let Some((image, next)) = heic_box(data, offset, end, b"ispe") {
            if image + 12 > next {
                return None;
            }
            let width = u32::from_be_bytes(data.get(image + 4..image + 8)?.try_into().ok()?);
            let height = u32::from_be_bytes(data.get(image + 8..image + 12)?.try_into().ok()?);
            if width == 0 || height == 0 {
                return None;
            }
            if u64::from(width) * u64::from(height) > MAX_HEIC_PIXELS {
                return Some(Err(Failure::TooLarge));
            }
            found = true;
            offset = next;
        }
        found.then_some(Ok(()))
    };
    parse().unwrap_or(Err(Failure::Unreadable))
}

#[cfg(target_os = "macos")]
fn decode_heic(bytes: &[u8]) -> Result<image::DynamicImage, Failure> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_foundation::{NSData, NSDictionary};
    // AppKit's ImageIO-backed decoder supports HEIC without another codec dependency.
    objc2::rc::autoreleasepool(|_| {
        let data = NSData::with_bytes(bytes);
        let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data)
            .ok_or(Failure::Unreadable)?;
        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }
        .ok_or(Failure::Unreadable)?;
        image::load_from_memory(unsafe { png.as_bytes_unchecked() })
            .map_err(|_| Failure::Unreadable)
    })
}

#[cfg(not(target_os = "macos"))]
fn decode_heic(_: &[u8]) -> Result<image::DynamicImage, Failure> {
    // The locked image codecs and Rune's Windows backend have no HEIC decoder.
    Err(Failure::Unreadable)
}

fn jpeg(image: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, Failure> {
    let mut rgb = image.to_rgb8();
    let rgba = image.to_rgba8();
    // KSG's JPEG fallback draws on a white matte, not black.
    for (pixel, source) in rgb.pixels_mut().zip(rgba.pixels()) {
        let alpha = u32::from(source[3]);
        for channel in 0..3 {
            pixel[channel] =
                ((u32::from(source[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
        .encode_image(&rgb)
        .map_err(|_| Failure::Unreadable)?;
    Ok(bytes)
}

pub fn prepare(
    name: &str,
    mime: &str,
    bytes: Vec<u8>,
    maximum: usize,
) -> Result<(String, Vec<u8>), Failure> {
    let heic = is_heic(name, mime);
    if heic && bytes.len() > MAX_SOURCE_BYTES {
        return Err(Failure::TooLarge);
    }
    if heic {
        validate_heic_dimensions(&bytes)?;
    }
    let mut decoded = if heic {
        decode_heic(&bytes)?
    } else {
        let dimensions = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|_| Failure::Unreadable)?
            .into_dimensions()
            .map_err(|_| Failure::Unreadable)?;
        let resize = matches!(mime, "image/png" | "image/jpeg" | "image/jpg")
            && fit_size(dimensions.0, dimensions.1).is_some();
        if !resize && bytes.len() <= maximum {
            return Ok((name.into(), bytes));
        }
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(Failure::TooLarge);
        }
        image::load_from_memory(&bytes).map_err(|_| Failure::Unreadable)?
    };
    let original_size = bytes.len();
    let mut fitted = None;
    if (heic || matches!(mime, "image/png" | "image/jpeg" | "image/jpg"))
        && let Some((width, height)) = fit_size(decoded.width(), decoded.height())
    {
        decoded = decoded.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        fitted = Some(if mime == "image/png" {
            let mut out = Cursor::new(Vec::new());
            decoded
                .write_to(&mut out, image::ImageFormat::Png)
                .map_err(|_| Failure::Unreadable)?;
            out.into_inner()
        } else {
            jpeg(&decoded, QUALITY_STEPS[0])?
        });
    } else if heic {
        fitted = Some(jpeg(&decoded, QUALITY_STEPS[0])?);
    }
    let jpeg_name = || {
        let base = name
            .rsplit_once('.')
            .filter(|(base, _)| !base.is_empty())
            .map_or(name, |(base, _)| base);
        format!("{base}.jpg")
    };
    if let Some(fitted) = &fitted
        && fitted.len() <= maximum
    {
        return Ok((if heic { jpeg_name() } else { name.into() }, fitted.clone()));
    }
    if original_size > MAX_SOURCE_BYTES {
        return Err(Failure::TooLarge);
    }
    // image exposes only lossless WebP encoding. Use KSG's JPEG codec fallback,
    // with its quality ladder, dimensions and data-URL budget.
    let base = decoded.width().max(decoded.height()).min(2000);
    for scale in [1.0, 0.75, 0.55] {
        let edge = (f64::from(base) * scale).round().max(1.0) as u32;
        let scale = f64::from(edge) / f64::from(decoded.width().max(decoded.height()));
        let resized = decoded.resize_exact(
            (f64::from(decoded.width()) * scale).round().max(1.0) as u32,
            (f64::from(decoded.height()) * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Lanczos3,
        );
        for quality in QUALITY_STEPS {
            let encoded = jpeg(&resized, quality)?;
            if "data:image/jpeg;base64,".len() + encoded.len().div_ceil(3) * 4 <= maximum / 3 * 4 {
                return Ok((jpeg_name(), encoded));
            }
        }
    }
    Err(Failure::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksg_attachment_type_cases() {
        for (name, mime, expected) in [
            ("diagram.svg", "image/svg+xml", Kind::File),
            ("diagram.SVG", "", Kind::File),
            ("diagram.svg", "image/jpeg", Kind::Image),
            ("photo.tiff", "image/tiff", Kind::File),
            ("report.pdf", "application/pdf", Kind::File),
            ("picture.bmp", "image/bmp", Kind::File),
            (
                "layers.psd",
                "image/vnd.adobe.photoshop",
                Kind::UnsupportedImage,
            ),
            ("photo.jpg", "image/jpeg", Kind::Image),
            ("export.dng.png", "image/png", Kind::Image),
            ("photo.webp", "image/webp", Kind::Image),
            ("photo.gif", "image/gif", Kind::Image),
            ("photo.heic", "", Kind::Image),
            ("photo.HEIF", "application/octet-stream", Kind::Image),
            ("photo.cr2", "image/x-canon-cr2", Kind::UnsupportedImage),
            ("photo.heic", "application/pdf", Kind::File),
        ] {
            assert_eq!(classify(name, mime), expected, "{name}");
        }
        for mime in ["image/png", "image/jpeg", "image/tiff", "image/dng", ""] {
            assert_eq!(classify("photo.DNG", mime), Kind::File);
            assert_eq!(file_mime("photo.DNG", mime), "image/x-adobe-dng");
        }
    }

    #[test]
    fn captioned_file_paste_only_claims_images_svg_and_dng() {
        assert!(claims_paste(
            "photo.dng",
            "application/octet-stream",
            "caption"
        ));
        assert!(claims_paste("diagram.svg", "image/svg+xml", "caption"));
        assert!(claims_paste("photo.png", "", "caption"));
        assert!(!claims_paste(
            "clipboard.rtf",
            "application/rtf",
            "Copied text"
        ));
        assert!(!claims_paste("photo.tiff", "image/tiff", "caption"));
        assert!(claims_paste("file.pdf", "application/pdf", ""));
    }

    #[test]
    fn source_resize_and_heic_metadata_limits() {
        assert_eq!(fit_size(4000, 3000), Some((2000, 1500)));
        assert_eq!(fit_size(3000, 4000), Some((1500, 2000)));
        assert_eq!(fit_size(2000, 2000), None);
        assert_eq!(fit_size(1, 5000), Some((1, 2000)));
        assert_eq!(
            validate_heic_dimensions(b"not a heic"),
            Err(Failure::Unreadable)
        );
    }

    #[test]
    fn png_is_fitted_before_the_byte_budget_and_gif_is_not_flattened() {
        let image = image::DynamicImage::new_rgba8(4000, 2);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (name, prepared) =
            prepare("wide.png", "image/png", png.into_inner(), 10 * 1024 * 1024).unwrap();
        assert_eq!(name, "wide.png");
        let image = image::load_from_memory(&prepared).unwrap();
        assert_eq!((image.width(), image.height()), (2000, 1));
        let mut gif = Cursor::new(Vec::new());
        image.write_to(&mut gif, image::ImageFormat::Gif).unwrap();
        let bytes = gif.into_inner();
        assert_eq!(
            prepare("wide.gif", "image/gif", bytes.clone(), 10 * 1024 * 1024)
                .unwrap()
                .1,
            bytes
        );
    }
}
