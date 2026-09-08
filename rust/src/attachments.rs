//! Structured local input; no remote fetching or automatic upload retries.
use crate::{Error, JvClient, Result, client::read_json, error::http_error};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Method,
    multipart::{Form, Part},
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContent {
    InputText { text: String },
    InputImage { image_url: String, detail: String },
    InputFile { file_id: String },
}

// Never expose image/base64 through derived Debug implementations.
impl std::fmt::Debug for InputContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InputImage { .. } => "InputImage([redacted])",
            Self::InputFile { .. } => "InputFile",
            Self::InputText { .. } => "InputText",
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StagedFile {
    pub id: String,
    pub object: String,
    pub bytes: u64,
    pub filename: String,
    pub media_type: String,
    pub created_at: i64,
    pub expires_at: i64,
}

fn read_local(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| Error::FileIo)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > limit {
        return Err(Error::InvalidInput("invalid or oversized local attachment"));
    }
    let file = std::fs::File::open(path).map_err(|_| Error::FileIo)?;
    if !file.metadata().map_err(|_| Error::FileIo)?.is_file() {
        return Err(Error::InvalidInput("regular local file required"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::FileIo)?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err(Error::InvalidInput("attachment size exceeded"));
    }
    Ok(bytes)
}

pub fn local_image(path: &Path, detail: &str) -> Result<InputContent> {
    if !matches!(detail, "auto" | "high") {
        return Err(Error::InvalidInput("image detail must be auto or high"));
    }
    let bytes = read_local(path, 5 * 1024 * 1024)?;
    let format = image::guess_format(&bytes).map_err(|_| Error::InvalidInput("invalid image"))?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err(Error::InvalidInput("unsupported image")),
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(80_000_000);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| Error::InvalidInput("invalid image content"))?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > 16_000_000 {
        return Err(Error::InvalidInput("image pixel limit exceeded"));
    }
    Ok(InputContent::InputImage {
        image_url: format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
        detail: detail.into(),
    })
}

fn local_file(path: &Path) -> Result<(String, &'static str, Vec<u8>)> {
    let suffix = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = match suffix.as_str() {
        "txt" => "text/plain",
        "md" => "text/markdown",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "csv" => "text/csv",
        "py" => "text/x-python",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return Err(Error::InvalidInput("unsupported structured file extension")),
    };
    let bytes = read_local(path, 10 * 1024 * 1024)?;
    if mime.starts_with("text/") || mime == "application/json" {
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error::InvalidInput("UTF-8 required"))?;
        if text.contains('\0') {
            return Err(Error::InvalidInput("NUL in text"));
        }
        if mime == "application/json" {
            serde_json::from_str::<serde_json::Value>(text)
                .map_err(|_| Error::InvalidInput("invalid JSON"))?;
        }
    } else if mime == "application/pdf" {
        if !bytes.starts_with(b"%PDF-")
            || !bytes[bytes.len().saturating_sub(2048)..]
                .windows(5)
                .any(|s| s == b"%%EOF")
        {
            return Err(Error::InvalidInput("invalid PDF signature"));
        }
    } else if !bytes.starts_with(b"PK\x03\x04") {
        return Err(Error::InvalidInput("invalid Office signature"));
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or(Error::InvalidInput("invalid filename"))?;
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(159 - suffix.len())
        .collect();
    Ok((format!("{safe}.{suffix}"), mime, bytes))
}

impl JvClient {
    /// Stage once. Retain the key and exact bytes after an uncertain result.
    /// The server performs full format/container admission, not just signatures.
    pub async fn stage_file(&self, path: &Path, key: &str) -> Result<StagedFile> {
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-'))
        {
            return Err(Error::InvalidInput("invalid upload idempotency key"));
        }
        let (name, mime, bytes) = local_file(path)?;
        let size = bytes.len() as u64;
        let part = Part::bytes(bytes)
            .file_name(name.clone())
            .mime_str(mime)
            .map_err(|_| Error::InvalidInput("invalid MIME"))?;
        let uncertain = || Error::ResponseSubmissionUncertain {
            status: None,
            idempotency_key: key.into(),
        };
        let response = self
            .authenticated(Method::POST, self.endpoint("/v1/files")?)?
            .header("Idempotency-Key", key)
            .multipart(Form::new().part("file", part))
            .send()
            .await
            .map_err(|_| uncertain())?;
        if !matches!(response.status().as_u16(), 200 | 201) {
            if response.status().is_client_error() && response.status().as_u16() != 408 {
                return Err(http_error(&response));
            }
            return Err(uncertain());
        }
        let value: StagedFile = read_json(response).await.map_err(|_| uncertain())?;
        if value.id.len() != 48
            || !value.id.starts_with("file_")
            || !value
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || value.object != "file"
            || value.bytes != size
            || value.filename != name
            || value.media_type != mime
            || value.expires_at <= value.created_at
        {
            return Err(uncertain());
        }
        Ok(value)
    }
}
