use std::{fs::File, path::PathBuf, io::Cursor};
use error_chain::error_chain;
use tempfile::{Builder, TempDir};

error_chain! {
    foreign_links {
        Io(std::io::Error);
        HttpRequest(reqwest::Error);
    }
}

pub fn download_mod(url: String) -> Result<(PathBuf, TempDir)> {
    let result = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(async {
        let tmp_dir = Builder::new().prefix("xrdmodman").tempdir()?;
        let response = reqwest::get(url).await?;

        let name: PathBuf;

        let mut dest = {
            let fname = response
                .url()
                .path_segments()
                .and_then(|segments| segments.last())
                .and_then(|name: &str| if name.is_empty() { None } else { Some(name) })
                .unwrap_or("tmp.bin");
            
            let fname = tmp_dir.path().join(fname);
            name = fname.clone();
            File::create(fname)?
        };

        let mut content =  Cursor::new(response.bytes().await?);
        std::io::copy(&mut content, &mut dest)?;

        Ok((name, tmp_dir))
    });

    result
}