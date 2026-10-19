use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FileState {
    pub page: usize,
    pub zoom: f32,
    pub fit: FitMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    #[default]
    Manual,
    BestFit,
    FitWidth,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
struct Store {
    files: HashMap<String, FileState>,
}

pub struct History {
    path: PathBuf,
    store: Store,
}

impl History {
    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let store = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        Self { path, store }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, file: &Path) -> Option<&FileState> {
        let key = normalize(file);
        self.store.files.get(&key)
    }

    pub fn save(&mut self, file: &Path, state: FileState) {
        let key = normalize(file);
        self.store.files.insert(key, state);
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.store) {
            let _ = fs::write(&self.path, json);
        }
    }
}

fn default_path() -> PathBuf {
    ProjectDirs::from("dev", "rift", "riftpdf")
        .map(|d| d.data_dir().join("history.json"))
        .unwrap_or_else(|| PathBuf::from("riftpdf-history.json"))
}

fn normalize(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_history() -> (PathBuf, History) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("riftpdf-history-test-{nanos}.json"));
        let _ = fs::remove_file(&path);
        (path.clone(), History::load_from(path))
    }

    #[test]
    fn roundtrip_save_and_load() {
        let (path, mut hist) = tmp_history();
        let pdf = std::env::temp_dir().join("riftpdf-hist-doc.pdf");
        fs::write(&pdf, b"%PDF-1.1\n").unwrap();

        hist.save(
            &pdf,
            FileState {
                page: 11,
                zoom: 1.25,
                fit: FitMode::FitWidth,
            },
        );

        let loaded = History::load_from(path.clone());
        let st = loaded.get(&pdf).expect("entry");
        assert_eq!(st.page, 11);
        assert!((st.zoom - 1.25).abs() < 1e-5);
        assert_eq!(st.fit, FitMode::FitWidth);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&pdf);
    }

    #[test]
    fn missing_file_yields_empty_store() {
        let path = std::env::temp_dir().join("riftpdf-history-missing-never.json");
        let _ = fs::remove_file(&path);
        let hist = History::load_from(path);
        assert!(hist.get(Path::new("/no/such/file.pdf")).is_none());
    }

    #[test]
    fn corrupt_json_falls_back_to_empty() {
        let path = std::env::temp_dir().join("riftpdf-history-corrupt.json");
        fs::write(&path, b"{not valid json").unwrap();
        let hist = History::load_from(path.clone());
        assert!(hist.get(Path::new("x.pdf")).is_none());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn fit_mode_serde_snake_case() {
        let st = FileState {
            page: 0,
            zoom: 1.0,
            fit: FitMode::BestFit,
        };
        let json = serde_json::to_string(&st).unwrap();
        assert!(json.contains("best_fit"));
        let back: FileState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, st);
    }
}
