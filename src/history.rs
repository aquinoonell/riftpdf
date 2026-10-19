use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    files: HashMap<String, FileState>,
}

pub struct History {
    path: PathBuf,
    store: Store,
}

impl History {
    pub fn load() -> Self {
        let path = ProjectDirs::from("dev", "rift", "riftpdf")
            .map(|d| d.data_dir().join("history.json"))
            .unwrap_or_else(|| PathBuf::from("riftpdf-history.json"));

        let store = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        Self { path, store }
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

fn normalize(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}
