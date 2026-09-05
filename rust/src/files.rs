use crate::{Error, Job, JvClient, ResponseFile, Result, error::http_error, types::validate_id};
use reqwest::{Method, header};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use url::Url;

const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
const MAX_FILES: usize = 10;

/// Accept only the exact job's relative content route; never signed/external URLs.
pub fn validate_download_url(base: &Url, job_id: &str, relative: &str) -> Result<Url> {
    validate_id(job_id)?;
    let prefix = format!("/v1/jobs/{job_id}/response-files/");
    let artifact = relative
        .strip_prefix(&prefix)
        .ok_or(Error::DownloadValidation("file URL is outside this job"))?;
    validate_id(artifact).map_err(|_| Error::DownloadValidation("invalid file route"))?;
    let url = base
        .join(relative)
        .map_err(|_| Error::DownloadValidation("invalid URL"))?;
    if url.origin() != base.origin()
        || url.path() != relative
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::DownloadValidation("file URL must be same-origin"));
    }
    Ok(url)
}

/// A single portable ASCII component, including protection from Windows devices.
pub fn safe_filename(value: &str, index: usize) -> String {
    let basename = value.rsplit(['/', '\\']).next().unwrap_or_default();
    let mapped: String = basename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut name: String = mapped.trim_matches([' ', '.']).chars().take(180).collect();
    name = name.trim_end_matches([' ', '.']).to_owned();
    if name.is_empty() {
        name = format!("jv-ai-output-{index}");
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        });
    if reserved {
        name.insert(0, '_');
    }
    name
}

fn validate_manifest(base: &Url, job: &Job) -> Result<Vec<Url>> {
    job.validate()?;
    if job.response.files.len() > MAX_FILES {
        return Err(Error::DownloadValidation("too many files"));
    }
    let mut total = 0;
    job.response
        .files
        .iter()
        .map(|item| {
            if item.size_bytes == 0 || item.size_bytes > MAX_FILE_BYTES {
                return Err(Error::DownloadValidation("invalid declared file size"));
            }
            total += item.size_bytes;
            if total > MAX_TOTAL_BYTES {
                return Err(Error::DownloadValidation("total download limit exceeded"));
            }
            validate_download_url(base, &job.id, &item.url)
        })
        .collect()
}

impl JvClient {
    /// Stream validated authenticated artifacts, using private temporary files and
    /// atomic no-clobber publication. Cancellation/errors remove partial files.
    /// Already completed files remain if a later download fails.
    pub async fn download_response_files(
        &self,
        job: &Job,
        destination: impl AsRef<Path>,
    ) -> Result<Vec<PathBuf>> {
        if !self.is_authenticated() {
            return Err(Error::NotAuthenticated);
        }
        let urls = validate_manifest(&self.base, job)?;
        if urls.is_empty() {
            return Ok(Vec::new());
        }
        let destination = destination.as_ref();
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(destination)
            .await
            .map_err(|_| Error::FileIo)?;
        let meta = tokio::fs::symlink_metadata(destination)
            .await
            .map_err(|_| Error::FileIo)?;
        if !meta.file_type().is_dir() {
            return Err(Error::DownloadValidation(
                "destination must be a non-symlink directory",
            ));
        }
        let destination = tokio::fs::canonicalize(destination)
            .await
            .map_err(|_| Error::FileIo)?;
        let mut paths = Vec::new();
        for (index, (item, url)) in job.response.files.iter().zip(urls).enumerate() {
            paths.push(
                self.download_one(item, url, &destination, index + 1)
                    .await?,
            );
        }
        Ok(paths)
    }

    async fn download_one(
        &self,
        item: &ResponseFile,
        url: Url,
        destination: &Path,
        index: usize,
    ) -> Result<PathBuf> {
        let mut response = self
            .authenticated(Method::GET, url)?
            .header(header::ACCEPT, "*/*")
            .header(header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if response.status() != 200 {
            return Err(http_error(&response));
        }
        if response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
        {
            return Err(Error::DownloadValidation(
                "encoded download is not supported",
            ));
        }
        if let Some(length) = response.headers().get(header::CONTENT_LENGTH) {
            let length = length
                .to_str()
                .ok()
                .filter(|v| !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit()))
                .and_then(|v| v.parse::<u64>().ok());
            if length != Some(item.size_bytes) {
                return Err(Error::DownloadValidation(
                    "Content-Length differs from manifest",
                ));
            }
        }
        let mut temporary = tempfile::Builder::new()
            .prefix(".jv-")
            .suffix(".part")
            .tempfile_in(destination)
            .map_err(|_| Error::FileIo)?;
        // NamedTempFile owns cleanup even if the async future is cancelled.
        let mut output = tokio::fs::File::from_std(temporary.reopen().map_err(|_| Error::FileIo)?);
        let mut written = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
            written = written
                .checked_add(chunk.len() as u64)
                .ok_or(Error::DownloadValidation("download size overflow"))?;
            if written > item.size_bytes || written > MAX_FILE_BYTES {
                return Err(Error::DownloadValidation("download exceeded declared size"));
            }
            output.write_all(&chunk).await.map_err(|_| Error::FileIo)?;
        }
        if written != item.size_bytes {
            return Err(Error::DownloadValidation(
                "download is shorter than declared size",
            ));
        }
        output.flush().await.map_err(|_| Error::FileIo)?;
        output.sync_all().await.map_err(|_| Error::FileIo)?;
        drop(output);
        let name = safe_filename(&item.name, index);
        let path = Path::new(&name);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("jv-ai-output");
        let suffix = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        for sequence in 0..1000 {
            let candidate = if sequence == 0 {
                name.clone()
            } else {
                format!("{stem}-{sequence}{suffix}")
            };
            let target = destination.join(candidate);
            match temporary.persist_noclobber(&target) {
                Ok(_) => return Ok(target),
                Err(error) => {
                    let collision = error.error.kind() == std::io::ErrorKind::AlreadyExists;
                    temporary = error.file;
                    if !collision {
                        return Err(Error::FileIo);
                    }
                }
            }
        }
        Err(Error::DownloadValidation("no unused filename available"))
    }
}
