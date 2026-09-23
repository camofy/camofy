//! Reads panel login captchas without a browser.
//!
//! The upstream image never leaves the process as-is: it is decoded, thresholded into a
//! black-glyph mask on white and upscaled with nearest-neighbour, which is what makes a
//! 100×24 captcha legible to a vision model. Only that transformed PNG is sent, as a data
//! URL, to the configured vision endpoint (`deepseek-flash` by default). No account name,
//! password, cookie or subscription URL is ever included in the request.

use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::io::Write;

/// Fixed service configuration for captcha reading. Absent means the feature is off.
#[derive(Clone)]
pub struct Vision {
    pub url: url::Url,
    pub key: String,
    pub model: String,
}

impl Vision {
    /// Enabled only when a key is configured. Endpoint and model have working defaults.
    /// The key is never logged and never leaves this struct.
    pub fn from_env() -> Result<Option<Self>> {
        let key = match std::env::var("CAMOFY_VISION_API_KEY") {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            _ => return Ok(None),
        };
        let url = url::Url::parse(
            &std::env::var("CAMOFY_VISION_API_URL")
                .unwrap_or_else(|_| "https://api.deepseek.com/chat/completions".into()),
        )?;
        ensure!(
            ["http", "https"].contains(&url.scheme())
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "CAMOFY_VISION_API_URL 必须是不含凭据的 HTTP(S) 地址"
        );
        let model = std::env::var("CAMOFY_VISION_MODEL")
            .unwrap_or_else(|_| "deepseek-flash".into())
            .trim()
            .to_string();
        ensure!(
            !model.is_empty() && model.len() <= 120,
            "CAMOFY_VISION_MODEL 无效"
        );
        Ok(Some(Self { url, key, model }))
    }
}

/// Plain image-to-text wording. Naming it a login challenge invites refusal and long
/// reasoning instead of the characters themselves.
const PROMPT: &str = "把这张图片里的文字转成文本，逐字输出。只输出识别到的字符，\
不要输出解释、标点、空格或其他任何内容。";
const MAX_IMAGE: usize = 256 * 1024;
const MAX_RESPONSE: usize = 64 * 1024;
const SCALE: usize = 6;
const THRESHOLD: u32 = 100;

/// Reads one captcha image and returns the code in upper case.
/// `private` follows CAMOFY_ALLOW_PRIVATE_EGRESS so a private/local deployment can reach a
/// self-hosted vision gateway; the endpoint itself is operator configuration, never user input.
pub async fn read(vision: &Vision, png: &[u8], private: bool) -> Result<String> {
    ensure!(
        !png.is_empty() && png.len() <= MAX_IMAGE,
        "验证码图片为空或过大"
    );
    // A decode failure must not lose the login attempt: send the original image instead.
    let prepared = prepare(png).unwrap_or_else(|_| png.to_vec());
    let image = format!("data:image/png;base64,{}", STANDARD.encode(&prepared));
    let messages = json!([{
        "role": "user",
        "content": [
            {"type": "text", "text": PROMPT},
            {"type": "image_url", "image_url": {"url": image}}
        ]
    }]);

    // Thinking would spend the whole budget on reasoning before naming the characters.
    let mut value = ask(vision, &messages, true, private).await?;
    if let Some(message) = value["error"]["message"].as_str()
        && message.to_ascii_lowercase().contains("thinking")
    {
        // A gateway that does not know the toggle is still usable.
        value = ask(vision, &messages, false, private).await?;
    }
    let message = &value["choices"][0]["message"];
    let content = message["content"].as_str().unwrap_or("");
    if let Some(code) = extract(content) {
        return Ok(code);
    }
    // A deployment that ignores the toggle answers inside the reasoning text instead.
    if let Some(code) = extract(message["reasoning_content"].as_str().unwrap_or("")) {
        return Ok(code);
    }
    if let Some(error) = value["error"]["message"].as_str() {
        bail!("验证码模型返回错误：{}", excerpt(error));
    }
    bail!(
        "验证码识别未返回可用字符（模型输出：{}）",
        excerpt(if content.trim().is_empty() {
            message["reasoning_content"].as_str().unwrap_or("(空)")
        } else {
            content
        })
    )
}

async fn ask(
    vision: &Vision,
    messages: &Value,
    disable_thinking: bool,
    private: bool,
) -> Result<Value> {
    let mut body = json!({
        "model": vision.model,
        "temperature": 0,
        "max_tokens": 128,
        "stream": false,
        "messages": messages,
    });
    if disable_thinking {
        body["thinking"] = json!({"type": "disabled"});
    }
    crate::security::post_json(&vision.url, Some(&vision.key), &body, MAX_RESPONSE, private).await
}

/// Short, single-line excerpt for diagnostics. Captcha text carries no account data.
fn excerpt(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect()
}

/// Captcha codes are 4–12 upper-case alphanumerics. A short pure answer wins; otherwise the
/// first code-like token is used, and only then the concatenation of every alphanumeric run.
pub fn extract(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let plain: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !trimmed.is_empty()
        && plain.len() == trimmed.chars().count()
        && (4..=12).contains(&plain.len())
    {
        return Some(plain);
    }
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_uppercase())
        .collect();
    let candidates: Vec<&String> = tokens
        .iter()
        .filter(|t| (4..=12).contains(&t.len()))
        .collect();
    // A captcha mixes letters and digits, so a mixed token beats prose such as "CODE".
    let code = candidates
        .iter()
        .copied()
        .filter(|t| {
            t.chars().any(|c| c.is_ascii_digit()) && t.chars().any(|c| c.is_ascii_alphabetic())
        })
        .max_by_key(|t| t.len())
        .or_else(|| candidates.iter().copied().max_by_key(|t| t.len()))?;
    Some(code.clone())
}

/// Decode → binarise → upscale → PNG. Kept separate so it can be exercised on its own.
pub fn prepare(png: &[u8]) -> Result<Vec<u8>> {
    let raster = decode(png)?;
    let wide = raster.width * SCALE;
    let high = raster.height * SCALE;
    ensure!(wide <= 4096 && high <= 4096, "captcha image too large");
    let mut scaled = Vec::with_capacity(wide * high);
    for y in 0..high {
        let row = (y / SCALE) * raster.width;
        for x in 0..wide {
            scaled.push(raster.glyphs[row + x / SCALE]);
        }
    }
    Ok(encode_gray(wide, high, &scaled))
}

/// Black-on-white glyph mask, one byte per pixel.
struct Raster {
    width: usize,
    height: usize,
    glyphs: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Result<Raster> {
    ensure!(
        bytes.len() > 8 && bytes[..8] == [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
        "not a PNG"
    );
    let mut pos = 8;
    let (mut width, mut height, mut depth, mut color, mut interlace) =
        (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut idat = Vec::new();
    loop {
        ensure!(pos + 12 <= bytes.len(), "truncated PNG");
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        ensure!(pos + 12 + len <= bytes.len(), "truncated PNG chunk");
        let kind = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + len];
        match kind {
            b"IHDR" => {
                ensure!(len == 13, "invalid IHDR");
                width = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                height = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
                depth = data[8];
                color = data[9];
                ensure!(data[10] == 0 && data[11] == 0, "unsupported PNG encoding");
                interlace = data[12];
            }
            b"PLTE" => palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    ensure!(!idat.is_empty(), "PNG has no image data");
    ensure!(interlace == 0, "interlaced PNG unsupported");
    ensure!(
        width > 0 && height > 0 && width <= 2048 && height <= 2048,
        "invalid PNG size"
    );
    let channels = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => bail!("unsupported PNG colour type"),
    };
    if color == 3 {
        ensure!(!palette.is_empty(), "palette PNG without PLTE");
    } else {
        ensure!(depth == 8, "unsupported PNG bit depth");
    }
    ensure!(matches!(depth, 1 | 2 | 4 | 8), "unsupported PNG bit depth");
    let bits = channels * depth as usize;
    let row_bytes = (width * bits).div_ceil(8);
    let step = bits.div_ceil(8);
    let mut raw = Vec::new();
    {
        use std::io::Read;
        flate2::read::ZlibDecoder::new(idat.as_slice()).read_to_end(&mut raw)?;
    }
    ensure!(raw.len() >= (row_bytes + 1) * height, "truncated PNG data");
    let mut glyphs = Vec::with_capacity(width * height);
    let mut previous = vec![0u8; row_bytes];
    for y in 0..height {
        let start = y * (row_bytes + 1);
        let mut line = raw[start + 1..start + 1 + row_bytes].to_vec();
        unfilter(raw[start], step, &previous, &mut line)?;
        for x in 0..width {
            let luminance = match color {
                3 => {
                    let index = sample(&line, x, depth) as usize;
                    let [r, g, b] = *palette
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("palette index out of range"))?;
                    luma(r, g, b)
                }
                0 => match depth {
                    8 => line[x] as u32,
                    _ => sample(&line, x, depth) as u32 * (255 / ((1 << depth) - 1)),
                },
                2 => luma(line[x * 3], line[x * 3 + 1], line[x * 3 + 2]),
                4 => line[x * 2] as u32,
                _ => luma(line[x * 4], line[x * 4 + 1], line[x * 4 + 2]),
            };
            glyphs.push(if luminance < THRESHOLD { 0 } else { 255 });
        }
        previous = line;
    }
    Ok(Raster {
        width,
        height,
        glyphs,
    })
}

fn sample(line: &[u8], x: usize, depth: u8) -> u8 {
    let bit = x * depth as usize;
    let byte = line[bit / 8];
    let shift = 8 - depth as usize - (bit % 8);
    (byte >> shift) & ((1u8 << depth) - 1)
}

fn luma(r: u8, g: u8, b: u8) -> u32 {
    (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000
}

fn unfilter(kind: u8, step: usize, previous: &[u8], line: &mut [u8]) -> Result<()> {
    for i in 0..line.len() {
        let left = if i >= step { line[i - step] } else { 0 };
        let up = previous[i];
        let up_left = if i >= step { previous[i - step] } else { 0 };
        line[i] = match kind {
            0 => line[i],
            1 => line[i].wrapping_add(left),
            2 => line[i].wrapping_add(up),
            3 => line[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
            4 => {
                let estimate = left as i32 + up as i32 - up_left as i32;
                let (pa, pb, pc) = (
                    (estimate - left as i32).abs(),
                    (estimate - up as i32).abs(),
                    (estimate - up_left as i32).abs(),
                );
                let predictor = if pa <= pb && pa <= pc {
                    left
                } else if pb <= pc {
                    up
                } else {
                    up_left
                };
                line[i].wrapping_add(predictor)
            }
            _ => bail!("unsupported PNG filter"),
        };
    }
    Ok(())
}

fn encode_gray(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height * (width + 1));
    for y in 0..height {
        raw.push(0);
        raw.extend_from_slice(&pixels[y * width..(y + 1) * width]);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).expect("in-memory zlib write");
    let compressed = encoder.finish().unwrap_or_default();

    let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    header.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut png);
    chunk(b"IDAT", &compressed, &mut png);
    chunk(b"IEND", &[], &mut png);
    png
}

fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes with an explicit filter so reconstruction is exercised, not just filter 0.
    fn encode_filtered(width: usize, height: usize, pixels: &[u8], filter: u8) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut previous = vec![0u8; width];
        for y in 0..height {
            let line = &pixels[y * width..(y + 1) * width];
            raw.push(filter);
            for i in 0..width {
                let left = if i > 0 { line[i - 1] } else { 0 };
                let up = previous[i];
                let up_left = if i > 0 { previous[i - 1] } else { 0 };
                raw.push(match filter {
                    0 => line[i],
                    1 => line[i].wrapping_sub(left),
                    2 => line[i].wrapping_sub(up),
                    3 => line[i].wrapping_sub(((left as u16 + up as u16) / 2) as u8),
                    _ => {
                        let estimate = left as i32 + up as i32 - up_left as i32;
                        let (pa, pb, pc) = (
                            (estimate - left as i32).abs(),
                            (estimate - up as i32).abs(),
                            (estimate - up_left as i32).abs(),
                        );
                        let predictor = if pa <= pb && pa <= pc {
                            left
                        } else if pb <= pc {
                            up
                        } else {
                            up_left
                        };
                        line[i].wrapping_sub(predictor)
                    }
                });
            }
            previous = line.to_vec();
        }
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        let mut header = Vec::new();
        header.extend_from_slice(&(width as u32).to_be_bytes());
        header.extend_from_slice(&(height as u32).to_be_bytes());
        header.extend_from_slice(&[8, 0, 0, 0, 0]);
        chunk(b"IHDR", &header, &mut png);
        chunk(b"IDAT", &compressed, &mut png);
        chunk(b"IEND", &[], &mut png);
        png
    }

    #[test]
    fn every_filter_reconstructs_the_original_scanlines() {
        // Already binary, so this test isolates filter reconstruction from the threshold.
        let pixels: Vec<u8> = vec![0, 255, 255, 0, 0, 255, 0, 255, 255, 0, 255, 0];
        for filter in 0..5u8 {
            let png = encode_filtered(4, 3, &pixels, filter);
            let raster = decode(&png).unwrap();
            assert_eq!((raster.width, raster.height), (4, 3), "filter {filter}");
            assert_eq!(raster.glyphs, pixels, "filter {filter}");
        }
    }

    #[test]
    fn the_threshold_decides_which_pixels_become_glyphs() {
        let pixels: Vec<u8> = vec![0, 40, 99, 100, 101, 255];
        let raster = decode(&encode_filtered(3, 2, &pixels, 0)).unwrap();
        assert_eq!(raster.glyphs, vec![0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn binarisation_and_upscale_produce_a_readable_mask() {
        // A 3×3 diagonal keeps one dark pixel per row, so block boundaries are checkable.
        let pixels: Vec<u8> = vec![0, 255, 255, 255, 0, 255, 255, 255, 0];
        let png = encode_filtered(3, 3, &pixels, 0);
        let prepared = prepare(&png).unwrap();
        let raster = decode(&prepared).unwrap();
        let width = 3 * SCALE;
        assert_eq!((raster.width, raster.height), (width, 3 * SCALE));
        assert_eq!(raster.glyphs[0], 0, "(0,0) stays dark");
        assert_eq!(raster.glyphs[SCALE - 1], 0, "the whole block is replicated");
        assert_eq!(raster.glyphs[SCALE], 255, "(0,1) stays white");
        assert_eq!(raster.glyphs[SCALE * width + SCALE], 0, "(1,1) stays dark");
        assert_eq!(raster.glyphs[2 * SCALE * width + 2 * SCALE], 0, "(2,2)");
        assert_eq!(raster.glyphs[2 * SCALE - 1], 255, "(0,2) stays white");
    }

    #[test]
    fn malformed_images_are_rejected_not_guessed() {
        assert!(decode(b"not a png at all").is_err());
        let good = encode_gray(2, 2, &[0, 255, 255, 0]);
        assert!(decode(&good).is_ok());
        assert!(decode(&good[..good.len() - 6]).is_err());
        let mut interlaced = good.clone();
        interlaced[8 + 8 + 12] = 1;
        assert!(decode(&interlaced).is_err());
        assert!(prepare(b"nope").is_err());
    }

    #[test]
    fn only_codes_survive_model_chatter() {
        assert_eq!(extract("4A7ZQ9").as_deref(), Some("4A7ZQ9"));
        assert_eq!(extract("  4a7zq9 \n").as_deref(), Some("4A7ZQ9"));
        assert_eq!(extract("The code is 4A7ZQ9.").as_deref(), Some("4A7ZQ9"));
        assert_eq!(extract("验证码：4A7ZQ9").as_deref(), Some("4A7ZQ9"));
        assert_eq!(extract("ABC").as_deref(), None);
        assert_eq!(extract("").as_deref(), None);
        assert_eq!(extract("图片无法辨认").as_deref(), None);
    }
}
