//! OS-provided user locations. Detection can read platform configuration; run it
//! on a blocking startup worker. It does not create or overwrite any directories.
use directories::{ProjectDirs, UserDirs};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformPaths {
    pub home: PathBuf,
    pub downloads: Option<PathBuf>,
    pub history: PathBuf,
    pub config: PathBuf,
}
impl PlatformPaths {
    pub fn detect() -> Result<Self, String> {
        let user =
            UserDirs::new().ok_or_else(|| "Unable to locate your home directory.".to_string())?;
        let project = ProjectDirs::from("dev", "Ulos", "Ulos")
            .ok_or_else(|| "Unable to locate application data.".to_string())?;
        let home = user.home_dir().to_owned();
        // Honor the configured directory, including an intentionally disabled one.
        // A conventional fallback is used only if it already exists.
        let downloads = user
            .download_dir()
            .map(Path::to_owned)
            .or_else(|| {
                let fallback = home.join("Downloads");
                fallback.is_dir().then_some(fallback)
            })
            .filter(|path| path.is_dir());
        Ok(Self {
            home,
            downloads,
            history: project.data_local_dir().join("history.sqlite"),
            config: project.config_dir().join("ulos.conf"),
        })
    }
    pub fn folder_label(&self, path: &Path) -> String {
        folder_label(path, Some(&self.home), self.downloads.as_deref())
    }
}

/// Friendly presentation only. Never use the lossy display label for file I/O;
/// retain the original PathBuf and expose its full display path in a tooltip.
pub fn folder_label(path: &Path, home: Option<&Path>, downloads: Option<&Path>) -> String {
    if downloads == Some(path) {
        return "Downloads".into();
    }
    if home == Some(path) {
        return "Home".into();
    }
    if let Some(downloads) = downloads
        && let Ok(relative) = path.strip_prefix(downloads)
        && !relative.as_os_str().is_empty()
    {
        return format!("Downloads/{}", relative.display());
    }
    if let Some(home) = home
        && let Ok(relative) = path.strip_prefix(home)
        && !relative.as_os_str().is_empty()
    {
        return format!("~/{}", relative.display());
    }
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
pub fn abbreviated_folder(path: &Path, home: &Path) -> String {
    folder_label(path, Some(home), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_locations_and_nested_labels() {
        let home = Path::new("/users/example");
        let downloads = home.join("Downloads");
        assert_eq!(folder_label(home, Some(home), Some(&downloads)), "Home");
        assert_eq!(
            folder_label(&downloads, Some(home), Some(&downloads)),
            "Downloads"
        );
        assert_eq!(
            folder_label(&downloads.join("Archives"), Some(home), Some(&downloads)),
            "Downloads/Archives"
        );
        assert_eq!(
            abbreviated_folder(&home.join("Documents"), home),
            "~/Documents"
        );
    }
    #[test]
    fn component_boundaries_not_string_prefixes() {
        assert_eq!(
            folder_label(
                Path::new("/users/example-other/Documents"),
                Some(Path::new("/users/example")),
                None
            ),
            "Documents"
        );
    }
    #[test]
    fn root_and_external_locations() {
        assert_eq!(folder_label(Path::new("/"), None, None), "/");
        assert_eq!(
            folder_label(Path::new("/mnt/data/Archives"), None, None),
            "Archives"
        );
    }
}
