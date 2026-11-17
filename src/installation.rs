use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

/// Only the activation callback can replace a running executable. Tests inject a
/// file operation on a disposable installation instead of calling self_replace.
pub fn replace_using(
    destination: &Path,
    executable: &[u8],
    activate: impl FnOnce(&Path) -> Result<()>,
) -> Result<String> {
    let directory = destination
        .parent()
        .context("Executable has no parent directory")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".boothop-update.lock"))
        .context(
            "Cannot write the installation directory. Run update with the required permissions.",
        )?;
    lock.try_lock().context("Another update is running")?;
    let staging = tempfile::Builder::new()
        .prefix(".boothop-update-")
        .tempdir_in(directory)?;
    let replacement = staging.path().join("boothop-new");
    let backup = staging.path().join("boothop-backup");
    fs::copy(destination, &backup)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&backup)?
        .sync_all()?;
    let original_hash = hash_file(&backup)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&replacement)?;
    file.write_all(executable)?;
    file.set_permissions(fs::metadata(destination)?.permissions())?;
    file.sync_all()?;
    drop(file);
    let result = activate(&replacement).and_then(|()| {
        ensure!(
            hash_file(destination)? == Sha256::digest(executable).to_vec(),
            "Replacement verification failed"
        );
        Ok(())
    });
    if let Err(error) = result {
        if hash_file(destination).ok().as_ref() != Some(&original_hash) {
            let recovery = fs::copy(&backup, destination).and_then(|_| {
                fs::OpenOptions::new()
                    .write(true)
                    .open(destination)?
                    .sync_all()
            });
            if let Err(recovery) = recovery {
                let saved = staging.keep();
                bail!(
                    "Replacement failed: {error:#}. Recovery failed: {recovery}. Restore the backup at {}",
                    saved.join("boothop-backup").display()
                );
            }
            if hash_file(destination).ok().as_ref() != Some(&original_hash) {
                let saved = staging.keep();
                bail!(
                    "Replacement and recovery verification failed. Restore the backup at {}",
                    saved.join("boothop-backup").display()
                );
            }
        }
        bail!("Replacement failed: {error:#}. The previous executable was restored.");
    }
    Ok("Update installed. Start Boothop again to use the new version.".into())
}

fn hash_file(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().to_vec())
}
