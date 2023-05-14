use std::{path::Path, io, fs};
use self_update::cargo_crate_version;

pub fn copy_recursively(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let filetype = entry.file_type()?;
        if filetype.is_dir() {
            copy_recursively(entry.path(), destination.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), destination.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn add1_char(c: char) -> char {
    std::char::from_u32(c as u32 + 1).unwrap_or(c)
}

pub fn add1_str(s: &str) -> String {
    s.chars().map(add1_char).collect()
}

pub fn update() -> Result<self_update::Status, self_update::errors::Error> {
    self_update::backends::github::Update::configure()
        .repo_owner("WistfulHopes")
        .repo_name("ggxrd-mod-manager")
        .bin_name("ggxrd-mod-manager.exe")
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
        .build()?
        .update()
}
