//! Managed media validation.
//!
//! Only raster formats the overlay renderer accepts are admitted: PNG, JPEG and
//! WebP. The format comes from the leading signature bytes, never from the
//! declared content type, and dimensions come from the header. Pixels are never
//! decoded, so a hostile image cannot exhaust memory in this process. SVG and
//! every other type are refused.

pub const MAX_BYTES: usize = 512 * 1024;
pub const MAX_DIMENSION: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub fn mime(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::Webp => "image/webp",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    pub format: Format,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    TooLarge,
    Empty,
    UnsupportedFormat,
    MalformedHeader,
    DimensionsOutOfRange,
}

/// Validates a candidate asset without decoding its pixels.
pub fn inspect(bytes: &[u8]) -> Result<Image, Rejection> {
    if bytes.is_empty() {
        return Err(Rejection::Empty);
    }
    if bytes.len() > MAX_BYTES {
        return Err(Rejection::TooLarge);
    }
    let (format, (width, height)) = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        (Format::Png, png_dimensions(bytes)?)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        (Format::Jpeg, jpeg_dimensions(bytes)?)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        (Format::Webp, webp_dimensions(bytes)?)
    } else {
        return Err(Rejection::UnsupportedFormat);
    };
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Rejection::DimensionsOutOfRange);
    }
    Ok(Image {
        format,
        width,
        height,
    })
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), Rejection> {
    // The first chunk after the signature must be IHDR with 13 bytes of data.
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return Err(Rejection::MalformedHeader);
    }
    let width = u32::from_be_bytes(
        bytes[16..20]
            .try_into()
            .map_err(|_| Rejection::MalformedHeader)?,
    );
    let height = u32::from_be_bytes(
        bytes[20..24]
            .try_into()
            .map_err(|_| Rejection::MalformedHeader)?,
    );
    Ok((width, height))
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), Rejection> {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return Err(Rejection::MalformedHeader);
        }
        let marker = bytes[i + 1];
        // Fill bytes may precede a marker.
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if length < 2 {
            return Err(Rejection::MalformedHeader);
        }
        // Start-of-frame markers carry the dimensions; DHT, JPG and DAC are not frames.
        let is_frame = (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_frame {
            if i + 9 > bytes.len() {
                return Err(Rejection::MalformedHeader);
            }
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Ok((width, height));
        }
        if marker == 0xDA {
            // Entropy-coded data starts before any frame header: refuse rather than scan it.
            return Err(Rejection::MalformedHeader);
        }
        i += 2 + length;
    }
    Err(Rejection::MalformedHeader)
}

fn webp_dimensions(bytes: &[u8]) -> Result<(u32, u32), Rejection> {
    match bytes.get(12..16) {
        Some(b"VP8 ") if bytes.len() >= 30 => {
            // Lossy: frame tag, 9d 01 2a start code, then 14-bit width and height.
            if bytes[23..26] != [0x9D, 0x01, 0x2A] {
                return Err(Rejection::MalformedHeader);
            }
            let width = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
            let height = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
            Ok((width, height))
        }
        Some(b"VP8L") if bytes.len() >= 25 => {
            if bytes[20] != 0x2F {
                return Err(Rejection::MalformedHeader);
            }
            let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
            Ok(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        Some(b"VP8X") if bytes.len() >= 30 => {
            let width = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]) + 1;
            let height = u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]) + 1;
            Ok((width, height))
        }
        _ => Err(Rejection::MalformedHeader),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes
    }

    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        // APP0 segment with length 16, then a baseline frame header.
        bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        bytes.extend_from_slice(&[0u8; 14]);
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = b"RIFF\x1A\x00\x00\x00WEBPVP8 \x0E\x00\x00\x00".to_vec();
        bytes.extend_from_slice(&[0x10, 0x02, 0x00]);
        bytes.extend_from_slice(&[0x9D, 0x01, 0x2A]);
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes
    }

    #[test]
    fn accepts_png_jpeg_and_webp_with_header_dimensions() {
        assert_eq!(
            inspect(&png(64, 32)).unwrap(),
            Image {
                format: Format::Png,
                width: 64,
                height: 32
            }
        );
        assert_eq!(
            inspect(&jpeg(320, 180)).unwrap(),
            Image {
                format: Format::Jpeg,
                width: 320,
                height: 180
            }
        );
        assert_eq!(
            inspect(&webp_lossy(128, 96)).unwrap(),
            Image {
                format: Format::Webp,
                width: 128,
                height: 96
            }
        );
    }

    #[test]
    fn detects_format_from_bytes_not_from_claims_and_refuses_svg() {
        assert_eq!(
            inspect(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"),
            Err(Rejection::UnsupportedFormat)
        );
        assert_eq!(
            inspect(b"GIF89a........"),
            Err(Rejection::UnsupportedFormat)
        );
    }

    #[test]
    fn enforces_size_and_dimension_bounds() {
        assert_eq!(inspect(&[]), Err(Rejection::Empty));
        assert_eq!(inspect(&vec![0u8; MAX_BYTES + 1]), Err(Rejection::TooLarge));
        assert_eq!(inspect(&png(0, 10)), Err(Rejection::DimensionsOutOfRange));
        assert_eq!(
            inspect(&png(MAX_DIMENSION + 1, 10)),
            Err(Rejection::DimensionsOutOfRange)
        );
    }

    #[test]
    fn truncated_or_inconsistent_headers_are_malformed() {
        let mut broken = png(10, 10);
        broken[12..16].copy_from_slice(b"XXXX");
        assert_eq!(inspect(&broken), Err(Rejection::MalformedHeader));
        assert_eq!(inspect(&png(10, 10)[..20]), Err(Rejection::MalformedHeader));
        let mut bad_webp = webp_lossy(10, 10);
        bad_webp[23] = 0;
        assert_eq!(inspect(&bad_webp), Err(Rejection::MalformedHeader));
    }
}
