//! Small facts about the machine that more than one crate needs.

use std::path::{Path, PathBuf};

/// Where `executable` resolves, as the shell would find it: an explicit path
/// as given, otherwise the first match on `PATH`, trying `PATHEXT`
/// extensions when the name has none.
pub fn resolve_executable(executable: &str) -> Option<PathBuf> {
    let given = Path::new(executable);
    if given.components().count() > 1 || given.is_absolute() {
        return given.is_file().then(|| given.to_path_buf());
    }
    let extensions: Vec<String> = if given.extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect()
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        extensions.iter().map(|ext| dir.join(format!("{executable}{ext}"))).find(|p| p.is_file())
    })
}
