//! Work that `async` commands hand off and await without blocking the tokio runtime: background
//! jobs for IO, and main-thread tasks for OLE and modal loops. Both run through
//! [`jerry_core::job::run_job`], so a panic fails only that job and the command returns
//! [`ErrorCode::Internal`](jerry_core::ErrorCode::Internal).

use jerry_core::JerryError;
use jerry_core::job::run_job;

/// Runs `job` on a background thread (tokio's blocking pool) and waits for its result.
pub async fn background<T: Send + 'static>(
    name: &'static str,
    job: impl FnOnce() -> Result<T, JerryError> + Send + 'static,
) -> Result<T, JerryError> {
    tauri::async_runtime::spawn_blocking(move || run_job(name, job))
        .await
        .unwrap_or_else(|error| {
            tracing::error!(job = name, %error, "background job didn't finish");
            Err(JerryError::internal())
        })
}

/// Runs `task` on the main thread through Jerry's own window (ADR-0002) and waits for its
/// result. For OLE, the clipboard and modal loops; never for IO.
pub async fn on_main<T: Send + 'static>(
    name: &'static str,
    task: impl FnOnce() -> Result<T, JerryError> + Send + 'static,
) -> Result<T, JerryError> {
    let (sender, result) = tokio::sync::oneshot::channel();
    jerry_win::main_thread::run_on_main(name, move || {
        let _ = sender.send(task());
    })?;
    // No result means the task panicked; `run_on_main` logged it.
    result.await.unwrap_or_else(|_| Err(JerryError::internal()))
}

#[cfg(test)]
mod tests {
    use jerry_core::ErrorCode;

    use super::*;

    #[test]
    fn background_jobs_return_results_and_survive_panics() {
        tauri::async_runtime::block_on(async {
            assert_eq!(background("adds", || Ok(2 + 2)).await, Ok(4));
            let panicked = background::<()>("panics", || panic!("deliberate panic")).await;
            assert_eq!(
                panicked.map_err(|error| error.code),
                Err(ErrorCode::Internal)
            );
            assert_eq!(
                background("after", || Ok("still running")).await,
                Ok("still running")
            );
        });
    }
}
