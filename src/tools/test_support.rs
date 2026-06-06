use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn temp_path(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("my_dev_tools_{unique}_{name}"))
}

pub fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

pub fn write_zst_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(path)?;
    let mut encoder = zstd::Encoder::new(file, 0)?;
    encoder.write_all(contents.as_bytes())?;
    encoder.finish()?;
    Ok(())
}

pub fn write_tar_zst_file(
    archive_path: &Path,
    entry_name: &str,
    contents: &str,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = archive_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(archive_path)?;
    let encoder = zstd::Encoder::new(file, 0)?;
    let mut builder = tar::Builder::new(encoder);

    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(contents.len() as u64);
    header.set_cksum();
    builder.append_data(&mut header, entry_name, contents.as_bytes())?;

    let encoder = builder.into_inner()?;
    encoder.finish()?;
    Ok(())
}
