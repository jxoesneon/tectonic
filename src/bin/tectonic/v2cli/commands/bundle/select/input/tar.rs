use anyhow::Result;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek},
    path::PathBuf,
};
use tar::Archive;
use tracing::info;

use super::BundleInput;

pub struct TarBundleInput {
    archive: Archive<File>,
    root: PathBuf,
    hash: String,
}

impl TarBundleInput {
    pub fn new(path: PathBuf, root: Option<PathBuf>) -> Result<Self> {
        let path = path.canonicalize()?;
        let mut file = File::open(&path)?;

        info!("computing hash of {}", path.display());

        let hash = {
            let mut hasher = Sha256::new();
            let _ = std::io::copy(&mut file, &mut hasher)?;
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .concat()
        };

        file.seek(std::io::SeekFrom::Start(0))?;
        Ok(Self {
            archive: Archive::new(file),
            root: root.unwrap_or(PathBuf::from("")),
            hash,
        })
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }
}

impl BundleInput for TarBundleInput {
    fn iter_files(&mut self) -> impl Iterator<Item = Result<(String, Box<dyn Read + '_>)>> {
        let root = self.root.clone();

        let entries = match self.archive.entries() {
            Ok(e) => e,
            Err(e) => {
                return Box::new(std::iter::once(Err(anyhow::Error::from(e))))
                    as Box<dyn Iterator<Item = Result<(String, Box<dyn Read + '_>)>>>
            }
        };

        Box::new(
            entries
                .map(move |x| {
                    let x = match x {
                        Ok(x) => x,
                        Err(e) => {
                            return Err(anyhow::Error::from(e).context("failed to read tar entry"))
                        }
                    };

                    if !x.header().entry_type().is_file() {
                        return Ok(None);
                    }

                    let path = match x.path() {
                        Ok(p) => p,
                        Err(e) => {
                            return Err(anyhow::Error::from(e).context("invalid tar entry path"))
                        }
                    };

                    if !path.starts_with(&root) {
                        return Ok(None);
                    }

                    let rel_path = match path.strip_prefix(&root) {
                        Ok(p) => match p.to_str() {
                            Some(s) => s.to_string(),
                            None => return Err(anyhow::anyhow!("invalid non-UTF8 path in tar")),
                        },
                        Err(e) => return Err(anyhow::Error::from(e)),
                    };

                    Ok(Some((rel_path, Box::new(x) as Box<dyn Read>)))
                })
                .filter_map(|x| x.transpose()),
        ) as Box<dyn Iterator<Item = Result<(String, Box<dyn Read + '_>)>>>
    }
}
