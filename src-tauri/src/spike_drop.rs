//! Spike #14: OLE drops into Jerry. Throwaway glue; what v0.2 keeps is `jerry_win::drop` and
//! the round trip here.
//!
//! The round trip, the point of the spike:
//! - `jerry_win::drop` calls [`Handler`] on the main thread for every `DragEnter`, `DragOver`,
//!   `DragLeave` and `Drop`. The page gets no mouse events during an OLE drag, so the handler
//!   sends the point as `dnd:over {seq, x, y, keys}` (through a forwarder thread, never from
//!   inside the COM call).
//! - The UI finds the element under the point (`elementFromPoint`), highlights it and answers
//!   with `dnd_answer {seq, target}`. The answer is stored here.
//! - `DragOver` returns at once with the effect for the last answer and the current keys. The
//!   answer is a frame behind the cursor; the keys are not, so Ctrl / Shift / Alt switch the
//!   cursor at once.
//! - On drop, the last answer is the target: the folder the user saw highlighted.
//!
//! Drops only ever copy into the lab, even when the user asks for a move: the source keeps its
//! files (we return COPY and "Performed DropEffect" COPY, so Explorer never deletes them).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, mpsc};
use std::time::Instant;

use jerry_core::sample;
use jerry_win::drag::DropEffect;
use jerry_win::drop::{
    self as ole, ClientPoint, Description, DragInfo, DropHandler, DropResult, Dropped, Effects,
    Feedback, Keys, VirtualFiles,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_specta::Event;

use crate::spike::{Effect, Lab, SpikeError, display_name, ms};

/// Mouse buttons and modifier keys, as the UI sees them.
#[derive(Clone, Copy, Debug, Default, Serialize, specta::Type)]
pub struct DndKeys {
    left: bool,
    right: bool,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl From<Keys> for DndKeys {
    fn from(keys: Keys) -> Self {
        Self {
            left: keys.left,
            right: keys.right,
            ctrl: keys.ctrl,
            shift: keys.shift,
            alt: keys.alt,
        }
    }
}

/// A drag entered the window. `x`, `y` are client pixels (divide by `devicePixelRatio`).
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:enter")]
pub struct DndEnter {
    session: u32,
    seq: u32,
    x: i32,
    y: i32,
    keys: DndKeys,
    /// CF_HDROP files and folders.
    files: u32,
    /// Virtual files (Outlook attachments, browser images).
    virtual_files: u32,
    /// The first few names, display only.
    names: Vec<String>,
    formats: Vec<String>,
    allowed: Vec<Effect>,
    async_capable: bool,
    from_jerry: bool,
}

/// The cursor moved or a key changed during a drag.
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:over")]
pub struct DndOver {
    session: u32,
    seq: u32,
    x: i32,
    y: i32,
    keys: DndKeys,
}

/// The drag left the window or was cancelled.
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:leave")]
pub struct DndLeave {
    session: u32,
}

/// The user dropped. With `menu`, the UI shows Copy here / Move here / Cancel and answers with
/// `dnd_choose`.
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:drop")]
pub struct DndDrop {
    session: u32,
    drop_id: u32,
    x: i32,
    y: i32,
    /// What the keys and the volumes asked for (the spike copies anyway).
    requested: Effect,
    menu: bool,
    items: u32,
    /// The target folder, `None` when the drop was refused.
    target: Option<u32>,
    /// Reading the data object inside `Drop`, and taking virtual file contents.
    read_ms: f64,
    take_ms: f64,
}

/// A drop's files are copied or saved.
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:done")]
pub struct DndDone {
    drop_id: u32,
    target: String,
    saved: Vec<String>,
    errors: Vec<String>,
    bytes: f64,
    ms: f64,
}

/// Round-trip measurements of one drag over Jerry.
#[derive(Clone, Debug, Serialize, specta::Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
#[tauri_specta(event_name = "dnd:stats")]
pub struct DndStats {
    session: u32,
    /// `DragOver` calls, and how many brought a new point or key state.
    overs: u32,
    emitted: u32,
    answered: u32,
    /// `DragOver` calls answered while the UI hadn't caught up with the newest point.
    stale: u32,
    rtt_median_ms: f64,
    rtt_p95_ms: f64,
    rtt_max_ms: f64,
    /// Time between `DragOver` calls.
    over_gap_median_ms: f64,
    over_gap_max_ms: f64,
}

/// What the UI found under the cursor.
#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DndAnswer {
    session: u32,
    seq: u32,
    target: Option<DndTarget>,
    labels: DropLabels,
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DndTarget {
    /// The folder a drop here goes to.
    id: u32,
    kind: TargetKind,
    /// The folder name for the drop description's `%1`.
    insert: String,
}

#[derive(Clone, Copy, Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TargetKind {
    Row,
    Tab,
    Crumb,
    Sidebar,
    Pane,
}

/// Drop description texts with `%1` for the folder name ("Move 3 items to %1").
#[derive(Clone, Debug, Default, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DropLabels {
    copy: String,
    moves: String,
    link: String,
}

/// A folder rectangle for the probe (CSS pixels of the page).
#[derive(Debug, Deserialize, specta::Type)]
pub struct LayoutRect {
    key: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Drop state shared between the drop target (main thread) and the commands.
#[derive(Default)]
pub struct Dnd {
    state: Mutex<DndState>,
    pending: Mutex<HashMap<u32, Pending>>,
    events: Mutex<Option<mpsc::Sender<Outgoing>>>,
}

#[derive(Default)]
struct DndState {
    session: u32,
    seq: u32,
    last: Option<(ClientPoint, Keys)>,
    /// Emitted points not answered yet, with when they were sent.
    sent: VecDeque<(u32, Instant)>,
    drag: Option<DragFacts>,
    answer: Option<Resolved>,
    volumes: HashMap<PathBuf, Option<PathBuf>>,
    stats: Stats,
    next_drop: u32,
}

struct DragFacts {
    sources: Vec<PathBuf>,
    source_volume: Option<PathBuf>,
    allowed: Effects,
    items: u32,
}

struct Resolved {
    seq: u32,
    target: Option<ResolvedTarget>,
    labels: DropLabels,
}

#[derive(Clone)]
struct ResolvedTarget {
    id: u32,
    kind: TargetKind,
    path: PathBuf,
    insert: String,
    same_volume: bool,
    /// Dropping a folder into itself or a subfolder, or moving files where they already are.
    forbidden: Forbidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Forbidden {
    No,
    IntoItself,
    SameFolder,
}

#[derive(Default)]
struct Stats {
    overs: u32,
    emitted: u32,
    answered: u32,
    stale: u32,
    rtts: Vec<f64>,
    gaps: Vec<f64>,
    last_over: Option<Instant>,
}

/// A right-button drop waiting for the menu.
struct Pending {
    files: Vec<PathBuf>,
    virtual_files: Option<VirtualFiles>,
    target: PathBuf,
}

enum Outgoing {
    Enter(DndEnter),
    Over(DndOver),
    Leave(DndLeave),
    Drop(DndDrop),
    Done(DndDone),
    Stats(DndStats),
}

thread_local! {
    /// The drop target registration. COM objects, so main thread only.
    static REGISTRATION: RefCell<Option<ole::Registration>> = const { RefCell::new(None) };
}

/// What `dnd_install` found and did.
#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    /// The window tree before registering: class, process, and `*` where a drop target was.
    before: Vec<String>,
    after: Vec<String>,
    registered: u32,
    /// `AllowExternalDrop(false)` worked.
    webview_drop_off: bool,
}

/// Registers Jerry's drop target on its window (again after a page reload). Runs on the main
/// thread through `with_webview`, which also hands over the WebView2 controller.
#[tauri::command]
#[specta::specta]
pub async fn dnd_install(
    app: AppHandle,
    window: WebviewWindow,
    dnd: State<'_, Dnd>,
) -> Result<InstallReport, SpikeError> {
    let hwnd = window.hwnd().map_err(|_| SpikeError::Os)?.0 as isize;
    let events = forwarder(&app, &dnd)?;
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    window
        .with_webview(move |webview| {
            let report = install(hwnd, app, events, &webview.controller());
            let _ = sender.try_send(report);
        })
        .map_err(|_| SpikeError::Os)?;
    receiver.recv().await.ok_or(SpikeError::Os)?
}

fn install<T: ole::ComInterface>(
    hwnd: isize,
    app: AppHandle,
    events: mpsc::Sender<Outgoing>,
    controller: &T,
) -> Result<InstallReport, SpikeError> {
    let before = tree_lines(hwnd);
    let webview_drop_off = match ole::disallow_webview_drop(controller) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "AllowExternalDrop(false) failed");
            false
        }
    };
    let handler: Rc<dyn DropHandler> = Rc::new(Handler { app, events });
    let registered = REGISTRATION.with(|slot| {
        // Revoke the old registration first, so the new one isn't revoked with it.
        slot.borrow_mut().take();
        let registration = ole::register(hwnd, handler).map_err(|error| {
            tracing::error!(%error, "RegisterDragDrop failed");
            SpikeError::Os
        })?;
        let count = registration.window_count() as u32;
        *slot.borrow_mut() = Some(registration);
        Ok::<_, SpikeError>(count)
    })?;
    let after = tree_lines(hwnd);
    tracing::info!(
        ?before,
        ?after,
        registered,
        webview_drop_off,
        "drop target installed"
    );
    Ok(InstallReport {
        before,
        after,
        registered,
        webview_drop_off,
    })
}

fn tree_lines(hwnd: isize) -> Vec<String> {
    ole::window_tree(hwnd)
        .into_iter()
        .map(|node| {
            format!(
                "{}{} ({}, pid {}){}",
                "  ".repeat(node.depth),
                node.class,
                if node.ours { "jerry" } else { "other" },
                node.process_id,
                if node.drop_target {
                    " *drop target*"
                } else {
                    ""
                }
            )
        })
        .collect()
}

/// The channel to the thread that emits drop events. Events are emitted from there, not from
/// inside OLE's COM calls on the main thread.
fn forwarder(app: &AppHandle, dnd: &Dnd) -> Result<mpsc::Sender<Outgoing>, SpikeError> {
    let mut slot = dnd.events.lock().map_err(|_| SpikeError::Os)?;
    if let Some(sender) = slot.as_ref() {
        return Ok(sender.clone());
    }
    let (sender, receiver) = mpsc::channel::<Outgoing>();
    let app = app.clone();
    std::thread::Builder::new()
        .name("dnd-events".into())
        .spawn(move || {
            for event in receiver {
                let result = match event {
                    Outgoing::Enter(event) => event.emit(&app),
                    Outgoing::Over(event) => event.emit(&app),
                    Outgoing::Leave(event) => event.emit(&app),
                    Outgoing::Drop(event) => event.emit(&app),
                    Outgoing::Done(event) => event.emit(&app),
                    Outgoing::Stats(event) => event.emit(&app),
                };
                if let Err(error) = result {
                    tracing::error!(%error, "emitting a drop event failed");
                }
            }
        })
        .map_err(|_| SpikeError::Os)?;
    *slot = Some(sender.clone());
    Ok(sender)
}

/// The UI's answer for a point it was sent.
#[tauri::command]
#[specta::specta]
pub fn dnd_answer(dnd: State<'_, Dnd>, lab: State<'_, Lab>, answer: DndAnswer) {
    let now = Instant::now();
    let Ok(mut state) = dnd.state.lock() else {
        return;
    };
    if answer.session != state.session || state.drag.is_none() {
        return;
    }
    if let Some(position) = state.sent.iter().position(|&(seq, _)| seq == answer.seq) {
        let sent = state.sent[position].1;
        state.stats.rtts.push(ms(now - sent));
        state.stats.answered += 1;
        state.sent.drain(..=position);
    }
    if state
        .answer
        .as_ref()
        .is_some_and(|last| last.seq > answer.seq)
    {
        return;
    }
    let target = answer.target.and_then(|target| {
        let path = lab.path(target.id)?;
        let volume = state
            .volumes
            .entry(path.clone())
            .or_insert_with(|| ole::volume_of(&path))
            .clone();
        let drag = state.drag.as_ref()?;
        let same_volume = volume.is_some() && volume == drag.source_volume;
        Some(ResolvedTarget {
            id: target.id,
            kind: target.kind,
            forbidden: forbidden(&drag.sources, &path),
            path,
            insert: target.insert,
            same_volume,
        })
    });
    state.answer = Some(Resolved {
        seq: answer.seq,
        target,
        labels: answer.labels,
    });
}

/// PRD: a file can't be dropped onto itself or into its own subfolder; moving files into the
/// folder they're in does nothing.
fn forbidden(sources: &[PathBuf], target: &Path) -> Forbidden {
    if sources.iter().any(|source| target.starts_with(source)) {
        Forbidden::IntoItself
    } else if !sources.is_empty() && sources.iter().all(|source| source.parent() == Some(target)) {
        Forbidden::SameFolder
    } else {
        Forbidden::No
    }
}

/// The effect and drop description for the last answer and the current keys.
fn feedback(state: &DndState, keys: Keys) -> (Feedback, Option<ResolvedTarget>) {
    let (Some(drag), Some(answer)) = (&state.drag, &state.answer) else {
        return (Feedback::NONE, None);
    };
    let Some(target) = &answer.target else {
        return (Feedback::NONE, None);
    };
    let mut effect = ole::choose_effect(keys, drag.allowed, target.same_volume);
    if target.forbidden == Forbidden::IntoItself
        || (target.forbidden == Forbidden::SameFolder && effect == DropEffect::Move)
    {
        effect = DropEffect::None;
    }
    let message = match effect {
        DropEffect::None => return (Feedback::NONE, Some(target.clone())),
        DropEffect::Copy => &answer.labels.copy,
        DropEffect::Move => &answer.labels.moves,
        DropEffect::Link => &answer.labels.link,
    };
    let feedback = Feedback {
        effect,
        description: Some(Description {
            icon: effect,
            message: message.clone(),
            insert: target.insert.clone(),
        }),
    };
    (feedback, Some(target.clone()))
}

fn effects_list(allowed: Effects) -> Vec<Effect> {
    [
        (allowed.copy, Effect::Copy),
        (allowed.moves, Effect::Move),
        (allowed.link, Effect::Link),
    ]
    .into_iter()
    .filter_map(|(on, effect)| on.then_some(effect))
    .collect()
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

struct Handler {
    app: AppHandle,
    events: mpsc::Sender<Outgoing>,
}

impl Handler {
    fn send(&self, event: Outgoing) {
        let _ = self.events.send(event);
    }

    /// Ends the drag's session: logs and reports its measurements.
    fn finish(&self, state: &mut DndState) {
        let stats = std::mem::take(&mut state.stats);
        let report = DndStats {
            session: state.session,
            overs: stats.overs,
            emitted: stats.emitted,
            answered: stats.answered,
            stale: stats.stale,
            rtt_median_ms: percentile(&stats.rtts, 0.5),
            rtt_p95_ms: percentile(&stats.rtts, 0.95),
            rtt_max_ms: percentile(&stats.rtts, 1.0),
            over_gap_median_ms: percentile(&stats.gaps, 0.5),
            over_gap_max_ms: percentile(&stats.gaps, 1.0),
        };
        tracing::info!(?report, "dnd session stats");
        state.drag = None;
        state.answer = None;
        state.sent.clear();
        state.last = None;
        self.send(Outgoing::Stats(report));
    }
}

impl DropHandler for Handler {
    fn trace(&self, call: &str) {
        tracing::debug!(call, "ole");
    }

    fn enter(&self, info: &DragInfo, at: ClientPoint, keys: Keys) -> Feedback {
        let dnd = self.app.state::<Dnd>();
        let Ok(mut state) = dnd.state.lock() else {
            return Feedback::NONE;
        };
        state.session += 1;
        state.seq += 1;
        state.stats = Stats::default();
        state.answer = None;
        state.last = Some((at, keys));
        state.sent.clear();
        let seq = state.seq;
        state.sent.push_back((seq, Instant::now()));
        state.stats.emitted += 1;
        let items = if info.files.is_empty() {
            info.virtual_files.len()
        } else {
            info.files.len()
        } as u32;
        state.drag = Some(DragFacts {
            sources: info.files.clone(),
            source_volume: info.files.first().and_then(|path| ole::volume_of(path)),
            allowed: info.allowed,
            items,
        });
        let names: Vec<String> = if info.files.is_empty() {
            info.virtual_files
                .iter()
                .map(|file| file.name.clone())
                .take(5)
                .collect()
        } else {
            info.files
                .iter()
                .map(|path| display_name(path))
                .take(5)
                .collect()
        };
        tracing::info!(
            session = state.session,
            files = info.files.len(),
            virtual_files = info.virtual_files.len(),
            formats = ?info.formats,
            allowed = ?info.allowed,
            async_capable = info.async_capable,
            from_jerry = info.from_self,
            ?names,
            "drag entered"
        );
        self.send(Outgoing::Enter(DndEnter {
            session: state.session,
            seq,
            x: at.x,
            y: at.y,
            keys: keys.into(),
            files: info.files.len() as u32,
            virtual_files: info.virtual_files.len() as u32,
            names,
            formats: info.formats.clone(),
            allowed: effects_list(info.allowed),
            async_capable: info.async_capable,
            from_jerry: info.from_self,
        }));
        Feedback::NONE
    }

    fn over(&self, at: ClientPoint, keys: Keys) -> Feedback {
        let dnd = self.app.state::<Dnd>();
        let Ok(mut state) = dnd.state.lock() else {
            return Feedback::NONE;
        };
        let now = Instant::now();
        state.stats.overs += 1;
        if let Some(last) = state.stats.last_over.replace(now) {
            state.stats.gaps.push(ms(now - last));
        }
        if state.last != Some((at, keys)) {
            state.last = Some((at, keys));
            state.seq += 1;
            let seq = state.seq;
            state.sent.push_back((seq, now));
            // Keep the backlog bounded if the UI stops answering.
            while state.sent.len() > 64 {
                state.sent.pop_front();
            }
            state.stats.emitted += 1;
            self.send(Outgoing::Over(DndOver {
                session: state.session,
                seq,
                x: at.x,
                y: at.y,
                keys: keys.into(),
            }));
        }
        if state
            .answer
            .as_ref()
            .is_none_or(|answer| answer.seq < state.seq)
        {
            state.stats.stale += 1;
        }
        feedback(&state, keys).0
    }

    fn leave(&self) {
        let dnd = self.app.state::<Dnd>();
        let Ok(mut state) = dnd.state.lock() else {
            return;
        };
        tracing::info!(session = state.session, "drag left");
        self.send(Outgoing::Leave(DndLeave {
            session: state.session,
        }));
        self.finish(&mut state);
    }

    fn dropped(&self, dropped: Dropped, at: ClientPoint, keys: Keys) -> DropResult {
        let refused = DropResult {
            effect: DropEffect::None,
            performed: DropEffect::None,
        };
        let dnd = self.app.state::<Dnd>();
        let Ok(mut state) = dnd.state.lock() else {
            return refused;
        };
        let (feedback, target) = feedback(&state, keys);
        let items = state.drag.as_ref().map_or(0, |drag| drag.items);
        state.next_drop += 1;
        let drop_id = state.next_drop;
        let session = state.session;
        let take_ms = dropped
            .virtual_files
            .as_ref()
            .map_or(0.0, |files| ms(files.take_time));
        let target = target.filter(|_| feedback.effect != DropEffect::None);
        let menu = dropped.right_button && target.is_some();
        tracing::info!(
            session,
            drop_id,
            requested = ?feedback.effect,
            menu,
            target = ?target.as_ref().map(|t| (t.id, t.kind, t.path.clone())),
            read_ms = ms(dropped.read_time),
            take_ms,
            "dropped"
        );
        self.send(Outgoing::Drop(DndDrop {
            session,
            drop_id,
            x: at.x,
            y: at.y,
            requested: feedback.effect.into(),
            menu,
            items,
            target: target.as_ref().map(|t| t.id),
            read_ms: ms(dropped.read_time),
            take_ms,
        }));
        self.finish(&mut state);
        drop(state);

        let Some(target) = target else {
            if let Some(files) = dropped.virtual_files {
                files.abandon();
            }
            return refused;
        };
        if menu {
            // The menu is Jerry's own (React), shown after Drop returns. The source is told
            // nothing happened; Jerry does the copy or move itself once the user picks.
            if let Ok(mut pending) = dnd.pending.lock() {
                pending.insert(
                    drop_id,
                    Pending {
                        files: dropped.files,
                        virtual_files: dropped.virtual_files,
                        target: target.path,
                    },
                );
            }
            return refused;
        }
        run(
            self.events.clone(),
            drop_id,
            Pending {
                files: dropped.files,
                virtual_files: dropped.virtual_files,
                target: target.path,
            },
        );
        // Spike: always a copy, so the source never deletes anything.
        DropResult {
            effect: DropEffect::Copy,
            performed: DropEffect::Copy,
        }
    }
}

/// The user's pick from the right-drag menu; `None` cancels.
#[tauri::command]
#[specta::specta]
pub fn dnd_choose(dnd: State<'_, Dnd>, drop_id: u32, choice: Option<Effect>) {
    let Some(pending) = dnd
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(&drop_id))
    else {
        return;
    };
    tracing::info!(drop_id, ?choice, "drop menu");
    let events = dnd.events.lock().ok().and_then(|events| events.clone());
    match (choice, events) {
        (Some(Effect::Copy | Effect::Move), Some(events)) => run(events, drop_id, pending),
        _ => {
            if let Some(files) = pending.virtual_files {
                files.abandon();
            }
        }
    }
}

/// Copies or saves a drop into its target folder on a worker thread, then reports `dnd:done`.
fn run(events: mpsc::Sender<Outgoing>, drop_id: u32, drop: Pending) {
    let spawned = std::thread::Builder::new()
        .name("dnd-copy".into())
        .spawn(move || {
            let start = Instant::now();
            let mut saved = Vec::new();
            let mut errors = Vec::new();
            let mut bytes = 0u64;
            for source in &drop.files {
                match sample::copy_into(source, &drop.target) {
                    Ok((path, size)) => {
                        saved.push(display_name(&path));
                        bytes += size;
                    }
                    Err(error) => errors.push(format!("{}: {error}", display_name(source))),
                }
            }
            if let Some(files) = drop.virtual_files {
                if let Err(error) = ole::init_worker() {
                    errors.push(format!("COM: {error}"));
                }
                files.save_each(|info, write| {
                    let Some(relative) = sample::safe_relative(&info.name) else {
                        errors.push(format!("{}: unsafe name, skipped", info.name));
                        return;
                    };
                    if info.is_dir {
                        if let Err(error) = std::fs::create_dir_all(drop.target.join(&relative)) {
                            errors.push(format!("{}: {error}", info.name));
                        }
                        return;
                    }
                    let path = sample::free_name(&drop.target, &relative);
                    let result = path
                        .parent()
                        .map_or(Ok(()), std::fs::create_dir_all)
                        .and_then(|()| write(&sample::part_path(&path)))
                        .and_then(|size| sample::commit_part(&path).map(|()| size));
                    match result {
                        Ok(size) => {
                            saved.push(display_name(&path));
                            bytes += size;
                        }
                        Err(error) => {
                            let _ = std::fs::remove_file(sample::part_path(&path));
                            errors.push(format!("{}: {error}", info.name));
                        }
                    }
                });
            }
            let done = DndDone {
                drop_id,
                target: display_name(&drop.target),
                saved,
                errors,
                bytes: bytes as f64,
                ms: ms(start.elapsed()),
            };
            tracing::info!(?done, "drop done");
            let _ = events.send(Outgoing::Done(done));
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "couldn't start the copy thread");
    }
}

/// Logs where the drop targets are on the page, for the probe that drives test drags.
#[tauri::command]
#[specta::specta]
pub fn lab_layout(dpr: f64, rects: Vec<LayoutRect>) {
    let line: Vec<String> = rects
        .iter()
        .map(|r| format!("{}={:.0},{:.0},{:.0},{:.0}", r.key, r.x, r.y, r.w, r.h))
        .collect();
    tracing::info!("layout dpr={dpr} {}", line.join(";"));
}
