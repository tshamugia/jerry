//! The Shell pool: 2–4 STA threads for Shell calls that can be slow or hang (icons, thumbnails,
//! Shell properties, `IFileOperation`). They never run on the main thread or the tokio runtime.
//!
//! A shell extension can hang, and one must not freeze Jerry: every job has a deadline. When it
//! passes, the job fails with `ERROR_TIMEOUT` and its worker is abandoned and replaced by a new
//! one. An abandoned thread exits if its call ever returns; its late result is dropped.
//!
//! Idle workers wait in `CoWaitForMultipleHandles`, which keeps pumping messages as an STA must.
//! The queue is last in, first out: the newest request is the one most likely for a row still on
//! screen. Past [`MAX_QUEUED`] waiting jobs, the oldest is dropped with `Cancelled`.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant};

use jerry_core::job::run_job;
use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::{CloseHandle, ERROR_TIMEOUT, HANDLE};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, COWAIT_DISPATCH_CALLS,
    COWAIT_DISPATCH_WINDOW_MESSAGES, CoInitializeEx, CoUninitialize, CoWaitForMultipleHandles,
};
use windows::Win32::System::Threading::{CreateSemaphoreW, INFINITE, ReleaseSemaphore};

/// Fewest and most workers (Tech PRD «Thread-ების მოდელი»).
const MIN_WORKERS: usize = 2;
const MAX_WORKERS: usize = 4;

/// Most jobs waiting for a worker. Past it the oldest waiting job fails with `Cancelled`: it was
/// for a row long scrolled away.
pub const MAX_QUEUED: usize = 512;

/// A pool of STA threads for Shell calls. Dropping it stops it: waiting jobs fail with
/// `Cancelled`, running ones finish and their workers exit.
pub struct ShellPool {
    pool: Arc<Pool>,
}

struct Pool {
    state: Mutex<State>,
    /// Wakes the watchdog when a job starts or the pool stops.
    watchdog: Condvar,
    /// Counts queued jobs; idle workers wait on it.
    ready: Semaphore,
    max_queued: usize,
}

struct State {
    /// Waiting jobs, oldest first; workers take the newest.
    queue: VecDeque<Job>,
    /// Live workers by id, with the job each one runs. A timed-out worker is removed.
    workers: HashMap<u64, Option<Running>>,
    next_id: u64,
    abandoned: usize,
    stopped: bool,
}

/// A job's two ways to end: `run` runs it and delivers its result, `fail` delivers an error
/// instead (timed out, dropped, pool stopped). Whichever comes first delivers; the other does
/// nothing.
struct Job {
    name: &'static str,
    timeout: Duration,
    run: Box<dyn FnOnce() + Send>,
    fail: Fail,
}

type Fail = Box<dyn FnOnce(JerryError) + Send>;

struct Running {
    name: &'static str,
    deadline: Instant,
    fail: Fail,
}

impl ShellPool {
    /// A pool sized to the machine: one worker per core, 2 to 4.
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism().map_or(MIN_WORKERS, |cores| cores.get());
        Self::with_workers(cores.clamp(MIN_WORKERS, MAX_WORKERS), MAX_QUEUED)
    }

    fn with_workers(workers: usize, max_queued: usize) -> Self {
        let pool = Arc::new(Pool {
            state: Mutex::new(State {
                queue: VecDeque::new(),
                workers: HashMap::new(),
                next_id: 1,
                abandoned: 0,
                stopped: false,
            }),
            watchdog: Condvar::new(),
            ready: Semaphore::new(),
            max_queued,
        });
        {
            let mut state = pool.state();
            for _ in 0..workers {
                spawn_worker(&pool, &mut state);
            }
        }
        let watched = pool.clone();
        let spawned = std::thread::Builder::new()
            .name("jerry-shell-watchdog".into())
            .spawn(move || watchdog(&watched));
        if let Err(error) = spawned {
            tracing::error!(%error, "the Shell pool watchdog didn't start: no job can time out");
        }
        Self { pool }
    }

    /// Queues `job` and returns at once. `done` gets its result on a pool thread, exactly once:
    /// the job's own result, an `Internal` error if it panicked, `ERROR_TIMEOUT` (as
    /// [`ErrorCode::Os`]) if it ran past `timeout`, or `Cancelled` if it was dropped from a full
    /// queue or the pool stopped.
    pub fn submit<T: Send + 'static>(
        &self,
        name: &'static str,
        timeout: Duration,
        job: impl FnOnce() -> Result<T, JerryError> + Send + 'static,
        done: impl FnOnce(Result<T, JerryError>) + Send + 'static,
    ) {
        type Done<T> = Box<dyn FnOnce(Result<T, JerryError>) + Send>;
        let done: Arc<Mutex<Option<Done<T>>>> = Arc::new(Mutex::new(Some(Box::new(done))));
        let take = |done: &Mutex<Option<Done<T>>>| {
            done.lock().unwrap_or_else(PoisonError::into_inner).take()
        };
        let delivers = done.clone();
        let run = Box::new(move || {
            let result = run_job(name, job);
            if let Some(done) = take(&delivers) {
                done(result);
            }
        });
        let fail = Box::new(move |error| {
            if let Some(done) = take(&done) {
                done(Err(error));
            }
        });
        let job = Job {
            name,
            timeout,
            run,
            fail,
        };

        let dropped = {
            let mut state = self.pool.state();
            if state.stopped {
                Some(job.fail)
            } else {
                state.queue.push_back(job);
                if state.queue.len() > self.pool.max_queued {
                    state.queue.pop_front().map(|oldest| oldest.fail)
                } else {
                    None
                }
            }
        };
        self.pool.ready.release(1);
        if let Some(fail) = dropped {
            fail(JerryError::new(ErrorCode::Cancelled));
        }
    }

    /// Runs `job` on the pool and waits for its result (see [`ShellPool::submit`]). Blocks the
    /// calling thread: call it from a background thread, never the main thread or tokio.
    pub fn run<T: Send + 'static>(
        &self,
        name: &'static str,
        timeout: Duration,
        job: impl FnOnce() -> Result<T, JerryError> + Send + 'static,
    ) -> Result<T, JerryError> {
        let (sender, result) = mpsc::channel();
        self.submit(name, timeout, job, move |result| {
            let _ = sender.send(result);
        });
        result
            .recv()
            .unwrap_or_else(|_| Err(JerryError::internal()))
    }
}

impl Default for ShellPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ShellPool {
    fn drop(&mut self) {
        let (waiting, workers) = {
            let mut state = self.pool.state();
            state.stopped = true;
            (std::mem::take(&mut state.queue), state.workers.len())
        };
        self.pool.watchdog.notify_all();
        self.pool.ready.release(workers);
        for job in waiting {
            (job.fail)(JerryError::new(ErrorCode::Cancelled));
        }
    }
}

impl Pool {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for the next job for worker `id` and marks it running. `None` once the pool stops.
    fn next_job(&self, id: u64) -> Option<Box<dyn FnOnce() + Send>> {
        loop {
            if !self.ready.wait() {
                return None;
            }
            let mut state = self.state();
            if state.stopped {
                return None;
            }
            // A dropped job leaves its count behind: then there's nothing to take.
            if let Some(job) = state.queue.pop_back() {
                let running = Running {
                    name: job.name,
                    deadline: Instant::now() + job.timeout,
                    fail: job.fail,
                };
                state.workers.insert(id, Some(running));
                self.watchdog.notify_one();
                return Some(job.run);
            }
        }
    }

    /// Marks worker `id` idle. False if it timed out meanwhile (it was replaced) or the pool
    /// stopped: then the worker exits.
    fn finish(&self, id: u64) -> bool {
        let mut state = self.state();
        let stopped = state.stopped;
        match state.workers.get_mut(&id) {
            Some(slot) => {
                *slot = None;
                !stopped
            }
            None => false,
        }
    }
}

fn spawn_worker(pool: &Arc<Pool>, state: &mut State) {
    let id = state.next_id;
    state.next_id += 1;
    state.workers.insert(id, None);
    let worker_pool = pool.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("jerry-shell-{id}"))
        .spawn(move || worker(&worker_pool, id));
    if let Err(error) = spawned {
        state.workers.remove(&id);
        tracing::error!(%error, "a Shell pool worker didn't start");
    }
}

fn worker(pool: &Pool, id: u64) {
    // SAFETY: a new thread enters an STA once; CoUninitialize below undoes it.
    let entered =
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if let Err(error) = entered.ok() {
        tracing::error!(%error, "a Shell pool worker couldn't enter an STA");
        pool.state().workers.remove(&id);
        return;
    }
    while let Some(run) = pool.next_job(id) {
        // `run` delivers its own result, and a panic in the job is caught inside it; this also
        // keeps a panicking `done` from ending the worker.
        let _ = run_job("shell pool job", || {
            run();
            Ok(())
        });
        if !pool.finish(id) {
            break;
        }
    }
    // SAFETY: pairs with the successful CoInitializeEx above, on the same thread.
    unsafe { CoUninitialize() };
}

/// Fails jobs that ran past their deadline and replaces their workers.
fn watchdog(pool: &Arc<Pool>) {
    let mut state = pool.state();
    loop {
        if state.stopped {
            return;
        }
        let now = Instant::now();
        let overdue: Vec<u64> = state
            .workers
            .iter()
            .filter(|(_, running)| running.as_ref().is_some_and(|job| job.deadline <= now))
            .map(|(&id, _)| id)
            .collect();
        let mut failed = Vec::new();
        for id in overdue {
            if let Some(Some(job)) = state.workers.remove(&id) {
                state.abandoned += 1;
                tracing::warn!(
                    job = job.name,
                    abandoned = state.abandoned,
                    "Shell job timed out; its worker is abandoned and replaced"
                );
                failed.push(job.fail);
                spawn_worker(pool, &mut state);
            }
        }
        if !failed.is_empty() {
            drop(state);
            for fail in failed {
                fail(JerryError::from_win32(ERROR_TIMEOUT.0));
            }
            state = pool.state();
            continue;
        }
        let next = state
            .workers
            .values()
            .flatten()
            .map(|job| job.deadline)
            .min();
        state = match next {
            Some(deadline) => {
                let wait = deadline.saturating_duration_since(now);
                let (state, _) = pool
                    .watchdog
                    .wait_timeout(state, wait)
                    .unwrap_or_else(PoisonError::into_inner);
                state
            }
            None => pool
                .watchdog
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// A Win32 semaphore: an STA can wait on it while pumping messages, which a `Condvar` can't.
struct Semaphore(HANDLE);

// SAFETY: a semaphore handle may be waited on and released from any thread.
unsafe impl Send for Semaphore {}
// SAFETY: as above; the handle itself never changes.
unsafe impl Sync for Semaphore {}

impl Semaphore {
    fn new() -> Self {
        // SAFETY: an unnamed semaphore with default security, starting at 0.
        let handle = unsafe { CreateSemaphoreW(None, 0, i32::MAX, None) };
        match handle {
            Ok(handle) => Self(handle),
            Err(error) => {
                // Without it no worker ever wakes, so jobs would wait forever: fail loudly.
                tracing::error!(%error, "the Shell pool's semaphore couldn't be created");
                Self(HANDLE::default())
            }
        }
    }

    fn release(&self, count: usize) {
        if count == 0 {
            return;
        }
        let count = i32::try_from(count).unwrap_or(i32::MAX);
        // SAFETY: our own semaphore handle; the previous count isn't needed.
        if let Err(error) = unsafe { ReleaseSemaphore(self.0, count, None) } {
            tracing::error!(%error, "the Shell pool's semaphore couldn't be released");
        }
    }

    /// Waits for a count while dispatching COM calls and window messages (STA rule). False if
    /// the wait failed.
    fn wait(&self) -> bool {
        let flags = (COWAIT_DISPATCH_CALLS.0 | COWAIT_DISPATCH_WINDOW_MESSAGES.0) as u32;
        // SAFETY: waits on our own semaphore handle, which lives as long as the pool.
        match unsafe { CoWaitForMultipleHandles(flags, INFINITE, &[self.0]) } {
            Ok(_) => true,
            Err(error) => {
                tracing::error!(%error, "a Shell pool worker's wait failed; it exits");
                false
            }
        }
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle is ours and closed once, here.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows::Win32::System::Com::{APTTYPE, APTTYPE_STA, APTTYPEQUALIFIER, CoGetApartmentType};

    use super::*;

    const LONG: Duration = Duration::from_secs(10);

    fn apartment() -> APTTYPE {
        let (mut kind, mut qualifier) = (APTTYPE::default(), APTTYPEQUALIFIER::default());
        // SAFETY: two valid out pointers.
        unsafe { CoGetApartmentType(&mut kind, &mut qualifier) }.expect("COM is initialized");
        kind
    }

    #[test]
    fn jobs_run_on_sta_workers() {
        let pool = ShellPool::new();
        let (kind, name) = pool
            .run("apartment", LONG, || {
                let name = std::thread::current().name().map(str::to_owned);
                Ok((apartment(), name))
            })
            .expect("runs");
        assert_eq!(kind, APTTYPE_STA);
        assert!(name.is_some_and(|name| name.starts_with("jerry-shell-")));
    }

    #[test]
    fn a_panic_fails_only_its_job() {
        let pool = ShellPool::with_workers(1, MAX_QUEUED);
        let failed = pool.run::<()>("panics", LONG, || panic!("deliberate panic"));
        assert_eq!(failed.map_err(|error| error.code), Err(ErrorCode::Internal));
        assert_eq!(pool.run("next", LONG, || Ok(7)), Ok(7));
    }

    #[test]
    fn a_hung_job_times_out_and_its_worker_is_replaced() {
        let pool = ShellPool::with_workers(2, MAX_QUEUED);
        let delivered = Arc::new(AtomicUsize::new(0));
        let (release, hung) = mpsc::channel::<()>();
        let hung = Arc::new(Mutex::new(hung));
        let (results, timed_out) = mpsc::channel();
        let started = Instant::now();
        for _ in 0..2 {
            let hung = hung.clone();
            let delivered = delivered.clone();
            let results = results.clone();
            let job = move || {
                let _ = hung.lock().map(|hung| hung.recv());
                Ok(())
            };
            pool.submit("hangs", Duration::from_millis(100), job, move |result| {
                delivered.fetch_add(1, Ordering::SeqCst);
                let _ = results.send(result);
            });
        }
        for _ in 0..2 {
            let result = timed_out.recv_timeout(LONG).expect("the job ends");
            let code = result.map_err(|error| error.code);
            assert_eq!(
                code,
                Err(ErrorCode::Os {
                    os: ERROR_TIMEOUT.0
                })
            );
        }
        assert!(started.elapsed() < Duration::from_secs(5));

        // Both workers hang, yet the replacements run new jobs.
        assert_eq!(pool.run("after", LONG, || Ok("replaced")), Ok("replaced"));

        // The hung calls return late: their results go nowhere.
        drop(release);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(delivered.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_newest_job_runs_first_and_a_full_queue_drops_the_oldest() {
        let pool = ShellPool::with_workers(1, 2);
        let (release, blocked) = mpsc::channel::<()>();
        pool.submit(
            "blocks",
            LONG,
            move || {
                let _ = blocked.recv();
                Ok(0)
            },
            |_| {},
        );
        // Let the worker take the blocking job before queueing the rest.
        std::thread::sleep(Duration::from_millis(50));
        let (results, order) = mpsc::channel();
        for n in 1..=3 {
            let results = results.clone();
            pool.submit(
                "queued",
                LONG,
                move || Ok(n),
                move |result| {
                    let _ = results.send(result.map_err(|error| error.code));
                },
            );
        }
        assert_eq!(
            order.recv_timeout(LONG).expect("dropped"),
            Err(ErrorCode::Cancelled),
            "the oldest of three in a queue of two"
        );
        drop(release);
        let rest: Vec<_> = (0..2)
            .map(|_| order.recv_timeout(LONG).expect("ran"))
            .collect();
        assert_eq!(rest, [Ok(3), Ok(2)]);
    }

    #[test]
    fn stopping_cancels_waiting_jobs() {
        let pool = ShellPool::with_workers(1, MAX_QUEUED);
        let (release, blocked) = mpsc::channel::<()>();
        pool.submit(
            "blocks",
            LONG,
            move || {
                let _ = blocked.recv();
                Ok(())
            },
            |_| {},
        );
        std::thread::sleep(Duration::from_millis(50));
        let (results, waiting) = mpsc::channel();
        pool.submit(
            "waits",
            LONG,
            || Ok(()),
            move |result| {
                let _ = results.send(result.map_err(|error| error.code));
            },
        );
        drop(pool);
        assert_eq!(
            waiting.recv_timeout(LONG).expect("ends"),
            Err(ErrorCode::Cancelled)
        );
        drop(release);
    }
}
