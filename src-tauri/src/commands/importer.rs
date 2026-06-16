use crate::core::{
    error::AppError,
    importer::{self, ImportCandidate, ImportResult},
    skill_store::SkillStore,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct ListCandidatesResult {
    pub candidates: Vec<ImportCandidate>,
}

#[tauri::command]
pub async fn list_import_candidates(
    workspace_path: String,
    store: State<'_, Arc<SkillStore>>,
) -> Result<ListCandidatesResult, AppError> {
    let _store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let workspace = PathBuf::from(&workspace_path);
        let candidates = importer::list_candidates(&workspace).map_err(AppError::io)?;
        Ok(ListCandidatesResult { candidates })
    })
    .await?
}

#[derive(Debug, Serialize)]
pub struct ImportAssetsResult {
    pub imported: Vec<ImportResult>,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SelectedAsset {
    pub asset_type: String,
    pub id_or_name: String,
    pub source_path: String,
    pub in_active_set: bool,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub tools: Option<Vec<String>>,
    pub codex_sandbox_mode: Option<String>,
    pub codex_reasoning_effort: Option<String>,
}

impl From<SelectedAsset> for ImportCandidate {
    fn from(s: SelectedAsset) -> Self {
        ImportCandidate {
            asset_type: s.asset_type,
            id_or_name: s.id_or_name,
            source_path: PathBuf::from(s.source_path),
            in_active_set: s.in_active_set,
            display_name: s.display_name,
            description: s.description,
            tools: s.tools,
            codex_sandbox_mode: s.codex_sandbox_mode,
            codex_reasoning_effort: s.codex_reasoning_effort,
        }
    }
}

#[tauri::command]
pub async fn import_selected_assets(
    selected: Vec<SelectedAsset>,
    store: State<'_, Arc<SkillStore>>,
) -> Result<ImportAssetsResult, AppError> {
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let candidates: Vec<ImportCandidate> = selected.into_iter().map(Into::into).collect();
        match importer::import_candidates(&candidates, &store) {
            Ok(imported) => Ok(ImportAssetsResult {
                imported,
                errors: vec![],
            }),
            Err(e) => Ok(ImportAssetsResult {
                imported: vec![],
                errors: vec![e.to_string()],
            }),
        }
    })
    .await?
}
