//! Shared task progress polling and cancellation commands.

use crate::state::tasks::{TaskProgress, TaskRegistry};
use tauri::State;

#[tauri::command]
pub fn task_progress(task_id: String, tasks: State<'_, TaskRegistry>) -> Option<TaskProgress> {
    tasks.get(&task_id)
}

#[tauri::command]
pub fn task_cancel(task_id: String, tasks: State<'_, TaskRegistry>) {
    tasks.cancel(&task_id);
}
