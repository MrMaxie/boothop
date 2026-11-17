use super::{Filesystem, MAX_ENTRIES, MAX_UKI, Partition, bounded};
use anyhow::{Context, Result, ensure};
use std::{
    collections::VecDeque,
    io::{Read, Seek},
};

pub(super) struct Ext(pub ext4_view::Ext4);
impl Filesystem for Ext {
    fn identity(&self) -> String {
        format!("ext:{}", self.0.uuid())
    }
    fn read(&mut self, _: u64, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        if !self.0.exists(path)? {
            return Ok(None);
        }
        let meta = self.0.metadata(path)?;
        ensure!(
            meta.file_type().is_regular_file(),
            "Boot file is not a regular file"
        );
        ensure!(
            meta.len() <= limit as u64,
            "Boot file exceeds the {limit}-byte inspection limit"
        );
        Ok(Some(bounded(self.0.open(path)?, limit)?))
    }
    fn list(&mut self, _: u64, path: &str) -> Result<Option<Vec<String>>> {
        if !self.0.exists(path)? {
            return Ok(None);
        }
        let mut names = Vec::new();
        for entry in self.0.read_dir(path)? {
            ensure!(
                names.len() < MAX_ENTRIES,
                "Boot directory inspection limit exceeded"
            );
            let entry = entry?;
            names.push(std::str::from_utf8(entry.file_name().as_ref())?.to_owned());
        }
        Ok(Some(names))
    }
}

pub(super) struct Btrfs<R>(ferrosys::btrfs::Reader<Partition<R>>);
impl<R: Read + Seek> Btrfs<R> {
    pub fn open(source: Partition<R>) -> Result<Self> {
        let options = ferrosys::OpenOptions::new().limits(
            ferrosys::Limits::new()
                .max_file_bytes(MAX_UKI as u64)
                .max_walk_entries(MAX_ENTRIES),
        );
        let reader = ferrosys::btrfs::Reader::open_with(source, &options)?;
        ensure!(
            reader.subvolumes().len() <= 256,
            "Btrfs subvolume inspection limit exceeded"
        );
        Ok(Self(reader))
    }
    // Resolve within each subvolume, including relative/absolute symlinks, as
    // it would appear mounted as a root filesystem. Never traverse host paths.
    fn node(&mut self, view: u64, path: &str) -> Result<Option<ferrosys::btrfs::Node>> {
        let root = self
            .0
            .subvolumes()
            .iter()
            .find(|v| v.id == view)
            .context("Missing subvolume")?
            .root_dir;
        let mut stack = vec![self.0.inode(view, root)?];
        let mut parts: VecDeque<_> = path.split('/').map(str::to_owned).collect();
        let mut hops = 0;
        while let Some(part) = parts.pop_front() {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                if stack.len() > 1 {
                    stack.pop();
                }
                continue;
            }
            let parent = stack.last().context("Missing path root")?;
            ensure!(parent.is_dir(), "Boot path component is not a directory");
            let Some(entry) = self.0.find_entry(parent, part.as_bytes())? else {
                return Ok(None);
            };
            let node = self.0.step(parent, &entry)?;
            if node.is_symlink() {
                hops += 1;
                ensure!(hops <= 40, "Too many symbolic links in boot path");
                let target = String::from_utf8(self.0.link_target(&node)?)?;
                if target.starts_with('/') {
                    stack.truncate(1);
                }
                for component in target.split('/').rev() {
                    parts.push_front(component.to_owned());
                }
            } else {
                stack.push(node);
            }
        }
        Ok(stack.pop())
    }
}
impl<R: Read + Seek> Filesystem for Btrfs<R> {
    fn identity(&self) -> String {
        format!(
            "btrfs:{}",
            uuid::Uuid::from_bytes(self.0.volume().superblock().fsid)
        )
    }
    fn views(&self) -> Vec<u64> {
        self.0.subvolumes().iter().map(|v| v.id).collect()
    }
    fn read(&mut self, view: u64, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some(node) = self.node(view, path)? else {
            return Ok(None);
        };
        ensure!(node.is_file(), "Boot file is not a regular file");
        ensure!(
            node.item.size <= limit as u64,
            "Boot file exceeds the {limit}-byte inspection limit"
        );
        Ok(Some(self.0.read_data(&node)?))
    }
    fn list(&mut self, view: u64, path: &str) -> Result<Option<Vec<String>>> {
        let Some(node) = self.node(view, path)? else {
            return Ok(None);
        };
        let entries = self.0.read_dir(&node)?;
        ensure!(
            entries.len() <= MAX_ENTRIES,
            "Boot directory inspection limit exceeded"
        );
        Ok(Some(
            entries
                .into_iter()
                .map(|e| String::from_utf8(e.name).map_err(Into::into))
                .collect::<Result<_>>()?,
        ))
    }
}

pub(super) struct Xfs<R> {
    fs: fstool::fs::xfs::Xfs,
    source: Partition<R>,
}
impl<R: Read + Seek + Send> Xfs<R> {
    pub fn open(mut source: Partition<R>) -> Result<Self> {
        Ok(Self {
            fs: fstool::fs::xfs::Xfs::open(&mut source)?,
            source,
        })
    }
    fn path(&mut self, path: &str) -> Result<Option<String>> {
        let mut parts: VecDeque<_> = path.split('/').map(str::to_owned).collect();
        let mut resolved = Vec::new();
        let mut hops = 0;
        while let Some(part) = parts.pop_front() {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." {
                resolved.pop();
                continue;
            }
            let directory = format!("/{}", resolved.join("/"));
            let entries = self.fs.list_path(&mut self.source, &directory)?;
            ensure!(
                entries.len() <= MAX_ENTRIES,
                "Boot directory inspection limit exceeded"
            );
            let Some(entry) = entries.iter().find(|e| e.name == part) else {
                return Ok(None);
            };
            if entry.kind == fstool::fs::EntryKind::Symlink {
                hops += 1;
                ensure!(hops <= 40, "Too many symbolic links in boot path");
                let target = self
                    .fs
                    .read_symlink(&mut self.source, &format!("{directory}/{part}"))?;
                if target.starts_with('/') {
                    resolved.clear();
                }
                for component in target.split('/').rev() {
                    parts.push_front(component.to_owned());
                }
            } else {
                resolved.push(part);
            }
        }
        Ok(Some(format!("/{}", resolved.join("/"))))
    }
}
impl<R: Read + Seek + Send> Filesystem for Xfs<R> {
    fn identity(&self) -> String {
        format!("xfs:{}", uuid::Uuid::from_bytes(self.fs.superblock().uuid))
    }
    fn read(&mut self, _: u64, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some(path) = self.path(path)? else {
            return Ok(None);
        };
        Ok(Some(bounded(
            self.fs.open_file_reader(&mut self.source, &path)?,
            limit,
        )?))
    }
    fn list(&mut self, _: u64, path: &str) -> Result<Option<Vec<String>>> {
        let Some(path) = self.path(path)? else {
            return Ok(None);
        };
        let entries = self.fs.list_path(&mut self.source, &path)?;
        ensure!(
            entries.len() <= MAX_ENTRIES,
            "Boot directory inspection limit exceeded"
        );
        Ok(Some(entries.into_iter().map(|e| e.name).collect()))
    }
}
