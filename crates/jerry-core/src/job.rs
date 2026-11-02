//! Background jobs run isolated: a panic fails only its own job and is logged, the app keeps
//! running (CLAUDE.md rule 11). Every pool and worker runs its jobs through [`run_job`].

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::error::JerryError;

/// Runs `job` and returns its result. If it panics, the panic is logged with the job's name and
/// the job fails with [`ErrorCode::Internal`](crate::ErrorCode::Internal); the calling thread
/// carries on.
///
/// Jobs count as unwind safe: what they share with other threads sits behind locks, and Jerry
/// reads locks through poisoning (`PoisonError::into_inner`).
pub fn run_job<T>(
    name: &str,
    job: impl FnOnce() -> Result<T, JerryError>,
) -> Result<T, JerryError> {
    catch_unwind(AssertUnwindSafe(job)).unwrap_or_else(|payload| {
        tracing::error!(
            job = name,
            panic = panic_message(payload.as_ref()),
            "job panicked and failed; Jerry keeps running"
        );
        Err(JerryError::internal())
    })
}

/// The message of a panic payload: `panic!` gives a `&str` or a `String`.
pub fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(no message)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[test]
    fn results_pass_through() {
        assert_eq!(run_job("ok", || Ok(7)), Ok(7));
        let error = JerryError::from_win32(5);
        assert_eq!(run_job::<()>("err", || Err(error.clone())), Err(error));
    }

    #[test]
    fn a_panic_fails_only_its_job() {
        let worker = std::thread::spawn(|| {
            let failed = run_job::<()>("panics", || panic!("deliberate panic"));
            let next = run_job("next", || Ok("still running"));
            (failed, next)
        });
        let (failed, next) = worker.join().expect("the worker thread survives the panic");
        assert_eq!(failed.map_err(|error| error.code), Err(ErrorCode::Internal));
        assert_eq!(next, Ok("still running"));
    }

    #[test]
    fn panic_messages_come_from_str_and_string_payloads() {
        let caught = |job: fn()| catch_unwind(job).expect_err("job panics");
        assert_eq!(panic_message(caught(|| panic!("plain")).as_ref()), "plain");
        assert_eq!(
            panic_message(caught(|| panic!("formatted {}", 42)).as_ref()),
            "formatted 42"
        );
        assert_eq!(
            panic_message(caught(|| std::panic::panic_any(3_u8)).as_ref()),
            "(no message)"
        );
    }
}
