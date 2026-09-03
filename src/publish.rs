use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn publish(path: &Path, text: &str) -> io::Result<bool> {
    if let Ok(cur) = fs::read_to_string(path) {
        if cur == text {
            return Ok(false);
        }
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)?;
    Ok(true)
}
