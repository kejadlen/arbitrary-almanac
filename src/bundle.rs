//! Workflows bundled into the binary, as `deno compile` does: `build` appends their sources to a
//! copy of this binary, followed by a trailer, and the binary reads them back at startup. Bundling
//! compiles no Rust, so any machine can produce a bundle for any platform's binary.
//!
//! The trailer is the last 24 bytes: the payload's length as a little-endian `u64`, then
//! [`MAGIC`]. The payload before it is the workflows as JSON.

use crate::host::Workflow;
use crate::{Catalog, Error};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

/// Marks a binary that carries a bundle.
const MAGIC: &[u8; 16] = b"arbitrary-almnc1";
const TRAILER: usize = 8 + MAGIC.len();

/// A workflow's source, under the file name its errors are labelled with.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bundled {
    pub name: String,
    pub source: String,
}

/// The workflows a deployment runs.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Bundle {
    pub workflows: Vec<Bundled>,
}

impl Bundle {
    /// Every `.fnl` file in `dir`, in file name order.
    pub fn read_dir(dir: &Path) -> Result<Self, Error> {
        let mut paths: Vec<std::path::PathBuf> = fs_err::read_dir(dir)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<_, _>>()?;
        paths.retain(|path| path.extension().is_some_and(|ext| ext == "fnl"));
        paths.sort();
        let workflows = paths
            .iter()
            .map(|path| {
                Ok(Bundled {
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    source: fs_err::read_to_string(path)?,
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self { workflows })
    }

    /// Loads every workflow. One that does not load refuses them all, so a deployment never runs
    /// with a workflow missing.
    pub fn load(&self, catalog: &Catalog) -> Result<Vec<Workflow>, Error> {
        self.workflows
            .iter()
            .map(|bundled| Workflow::load(catalog, &bundled.name, &bundled.source))
            .collect()
    }

    /// `binary` with this bundle appended, in place of any bundle it already carried.
    pub fn append_to(&self, binary: &[u8]) -> Vec<u8> {
        let payload = serde_json::to_vec(self).unwrap_or_default();
        let length = u64::try_from(payload.len()).unwrap_or_default();
        let mut bundled = unbundled(binary).to_vec();
        bundled.extend_from_slice(&payload);
        bundled.extend_from_slice(&length.to_le_bytes());
        bundled.extend_from_slice(MAGIC);
        bundled
    }

    /// The bundle at the end of the binary at `path`, if it carries one.
    pub fn of_binary(path: &Path) -> Result<Option<Self>, Error> {
        let mut file = fs_err::File::open(path)?;
        let size = file.metadata()?.len();
        let Some(at) = size.checked_sub(TRAILER as u64) else {
            return Ok(None);
        };
        file.seek(SeekFrom::Start(at))?;
        let mut trailer = [0; TRAILER];
        file.read_exact(&mut trailer)?;
        let (length, magic) = trailer.split_at(8);
        if magic != MAGIC {
            return Ok(None);
        }
        let length = u64::from_le_bytes(length.try_into().unwrap_or_default());
        let start = at.checked_sub(length).ok_or_else(|| corrupt(path))?;
        file.seek(SeekFrom::Start(start))?;
        let mut payload = vec![];
        file.take(length).read_to_end(&mut payload)?;
        serde_json::from_slice(&payload)
            .map(Some)
            .map_err(|_| corrupt(path))
    }
}

/// `binary` without the bundle it carries, if any.
fn unbundled(binary: &[u8]) -> &[u8] {
    let Some((rest, trailer)) = binary.split_last_chunk::<TRAILER>() else {
        return binary;
    };
    let (length, magic) = trailer.split_at(8);
    let length = u64::from_le_bytes(length.try_into().unwrap_or_default());
    let start = usize::try_from(length)
        .ok()
        .and_then(|length| rest.len().checked_sub(length));
    match (magic == MAGIC, start) {
        (true, Some(start)) => rest.get(..start).unwrap_or(binary),
        _ => binary,
    }
}

fn corrupt(path: &Path) -> Error {
    Error::Bundle {
        path: path.display().to_string(),
    }
}

/// Checks the workflows in `dir`, then writes `binary` with them bundled to `out`, executable.
/// Nothing is written unless every workflow loads.
pub fn build(catalog: &Catalog, dir: &Path, binary: &Path, out: &Path) -> Result<Bundle, Error> {
    let bundle = Bundle::read_dir(dir)?;
    bundle.load(catalog)?;
    let bytes = bundle.append_to(&fs_err::read(binary)?);
    fs_err::write(out, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs_err::set_permissions(out, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(bundle)
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
