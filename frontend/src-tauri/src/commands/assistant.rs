//! Assistant runtime and tool-service commands.

use crate::error::ApiError;
use crate::state::assistant::{AssistantServicesConfig, AssistantServicesState};
use tauri::State;

#[tauri::command]
pub fn assistant_init_services(
    config: AssistantServicesConfig,
    services: State<'_, AssistantServicesState>,
) -> Result<(), ApiError> {
    services
        .initialize(config)
        .then_some(())
        .ok_or_else(|| ApiError::Message("Failed to initialize assistant services".to_string()))
}
