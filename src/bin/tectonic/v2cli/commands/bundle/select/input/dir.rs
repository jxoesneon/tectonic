use super::BundleInput;
use anyhow::{Context, Result};
use std::{
    fs::{self},
    io::Read,
    path::PathBuf,
};
use walkdir::WalkDir;

pub struct DirBundleInput {
    dir: PathBuf,
}

impl DirBundleInput {
    pub fn new(dir: PathBuf) -> Result<Self> {
        Ok(Self {
            dir: dir
                .canonicalize()
                .context("failed to canonicalize bundle directory")?,
        })
    }
}

impl BundleInput for DirBundleInput {
    fn iter_files(&mut self) -> impl Iterator<Item = Result<(String, Box<dyn Read + '_>)>> {
        WalkDir::new(&self.dir)
            .into_iter()
            .filter_map(|x| match x {
                Err(_) => Some(x),
                Ok(x) => {
                    if !x.file_type().is_file() {
                        None
                    } else {
                        Some(Ok(x))
                    }
                }
            })
            .map(move |x| match x {
                Ok(x) => {
                    let path = x.into_path();
                    let path = match path.canonicalize() {
                        Ok(p) => p,
                        Err(e) => {
                            return Err(
                                anyhow::Error::from(e).context("failed to canonicalize path")
                            )
                        }
                    };
                    let path = match path.strip_prefix(&self.dir) {
                        Ok(p) => p,
                        Err(e) => {
                            return Err(anyhow::Error::from(e).context("failed to strip prefix"))
                        }
                    };
                    let path = match path.to_str() {
                        Some(s) => s.to_string(),
                        None => return Err(anyhow::anyhow!("invalid non-UTF8 path")),
                    };

                    Ok((
                        path.clone(),
                        Box::new(fs::File::open(self.dir.join(path))?) as Box<dyn Read>,
                    ))
                }
                Err(e) => Err(anyhow::Error::from(e)),
            })
    }
}
