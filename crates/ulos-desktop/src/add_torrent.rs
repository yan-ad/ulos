//! Local-only torrent confirmation. This view never creates a download task or a session.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::{
        Disableable,
        button::Button,
        description_list::{DescriptionItem, DescriptionList},
        form::{Field, Form},
        group_box::GroupBox,
        h_flex,
        input::{Input, InputEvent, InputState},
        label::Label,
        radio::{Radio, RadioGroup},
        separator::Separator,
        toolbar::Toolbar,
        v_flex,
    },
    *,
};
use ulos_core::{
    naming::sanitize_filename,
    torrent::{TorrentOptions, TorrentSource},
    transfer::CollisionPolicy,
};

pub struct TorrentRequest {
    pub source: TorrentSource,
    pub destination: PathBuf,
    pub start_now: bool,
    pub options: TorrentOptions,
}
pub struct DismissTorrent;

const PRIVACY: &str = "Confirm before contacting trackers and peers. Download later may start in a running queue. DHT, local discovery, port forwarding, payload upload and seeding are disabled.";
const COLLISIONS: [CollisionPolicy; 2] = [CollisionPolicy::Reject, CollisionPolicy::Rename];
const FILE_PREVIEW_LIMIT: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetState {
    Unchecked,
    Checking,
    Missing,
    Exists,
    Blocked,
    Error,
}
impl TargetState {
    fn inspect(path: &Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() || metadata.is_dir() => Self::Exists,
            Ok(_) => Self::Blocked, // symlinks (including dangling ones) and special files
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Error,
        }
    }
}

// Dropping a Tokio JoinHandle alone detaches it. Keep cancellation owned by the draft,
// even if the GPUI observer has not yet been polled.
struct Worker(tokio::task::AbortHandle);
impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct AddTorrent {
    pub source: Option<TorrentSource>,
    pub destination: Entity<InputState>,
    pub choosing_path: bool,
    pub options: TorrentOptions,
    default_directory: Option<PathBuf>,
    suggested_path: Option<String>,
    source_filename: Option<String>,
    source_error: Option<String>,
    loading: bool,
    runtime: tokio::runtime::Handle,
    generation: u64,
    loader: Option<Worker>,
    _load_task: Option<Task<()>>,
    target_generation: u64,
    target: TargetState,
    checked_target: Option<PathBuf>,
    target_worker: Option<Worker>,
    _target_task: Option<Task<()>>,
    validation_requested: bool,
    _subscriptions: Vec<Subscription>,
    _prompt: Option<Task<()>>,
}
impl EventEmitter<TorrentRequest> for AddTorrent {}
impl EventEmitter<DismissTorrent> for AddTorrent {}

impl AddTorrent {
    pub fn new(
        source_path: Option<PathBuf>,
        default_directory: Option<PathBuf>,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let destination = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Full destination file or root folder path")
        });
        let subscription = cx.subscribe_in(&destination, window, |view, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                view.check_target(window, cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                view.submit(true, cx);
            }
            cx.notify();
        });
        let mut view = Self {
            source: None,
            destination,
            choosing_path: false,
            options: TorrentOptions::default(),
            default_directory,
            suggested_path: None,
            source_filename: None,
            source_error: None,
            loading: false,
            runtime,
            generation: 0,
            loader: None,
            _load_task: None,
            target_generation: 0,
            target: TargetState::Unchecked,
            checked_target: None,
            target_worker: None,
            _target_task: None,
            validation_requested: false,
            _subscriptions: vec![subscription],
            _prompt: None,
        };
        if let Some(path) = source_path {
            view.set_source_path(path, window, cx);
        }
        view
    }

    /// Replace the draft, invalidating both old metadata and any conflict choice.
    pub fn set_source_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        self.loader = None;
        self._load_task = None;
        self.source = None;
        self.loading = false;
        self.source_error = None;
        self.source_filename = path
            .file_name()
            .map(|name| name.to_string_lossy().chars().take(160).collect());
        self.validation_requested = false;
        self.invalidate_target();
        if !is_torrent_path(&path) {
            self.source_error = Some("Choose a single local .torrent file.".into());
            cx.notify();
            return;
        }
        self.loading = true;
        let generation = self.generation;
        // Core load is bounded local parsing, not tracker/peer discovery.
        let worker = self.runtime.spawn(ulos_core::torrent::load(path));
        self.loader = Some(Worker(worker.abort_handle()));
        self._load_task = Some(cx.spawn_in(window, async move |view, cx| {
            // Do not await a pending Tokio handle on GPUI's test executor.
            while !worker.is_finished() {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
            }
            let result = worker.await;
            let _ = view.update_in(cx, |view, window, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                view.loader = None;
                match result {
                    Ok(Ok(source)) => {
                        view.source = Some(source);
                        view.suggest_destination(window, cx);
                        view.check_target(window, cx);
                    }
                    Ok(Err(error)) => {
                        view.source_error = Some(error.to_string().chars().take(240).collect())
                    }
                    Err(_) => {
                        view.source_error =
                            Some("Could not read this torrent. Choose the file again.".into())
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn destination_path(&self, cx: &App) -> Option<PathBuf> {
        sanitized_destination(self.destination.read(cx).value().as_ref())
    }

    fn suggest_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.destination.read(cx).value().to_string();
        if !current.trim().is_empty() && self.suggested_path.as_deref() != Some(&current) {
            return;
        }
        let (Some(directory), Some(source)) = (&self.default_directory, &self.source) else {
            return;
        };
        let path = directory.join(sanitize_filename(source.name()));
        if let Some(value) = path.to_str() {
            let value = value.to_owned();
            self.destination
                .update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            self.suggested_path = Some(value);
        }
    }

    fn invalidate_target(&mut self) {
        self.target_generation = self.target_generation.wrapping_add(1);
        self.target_worker = None;
        self._target_task = None;
        self.target = TargetState::Unchecked;
        self.checked_target = None;
        self.options.collision = CollisionPolicy::Reject;
    }

    fn check_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_target();
        let Some(path) = self.destination_path(cx) else {
            return;
        };
        self.target = TargetState::Checking;
        let generation = self.target_generation;
        let expected = path.clone();
        let worker = self
            .runtime
            .spawn_blocking(move || TargetState::inspect(&path));
        self.target_worker = Some(Worker(worker.abort_handle()));
        self._target_task = Some(cx.spawn_in(window, async move |view, cx| {
            while !worker.is_finished() {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
            }
            let state = worker.await.unwrap_or(TargetState::Error);
            let _ = view.update_in(cx, |view, _, cx| {
                if generation != view.target_generation
                    || view.destination_path(cx).as_ref() != Some(&expected)
                {
                    return;
                }
                view.target_worker = None;
                view.target = state;
                view.checked_target = Some(expected);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_destination(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = path.to_str() {
            self.suggested_path = None;
            self.destination
                .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx));
            self.check_target(window, cx);
        } else {
            self.invalidate_target();
            self.validation_requested = true;
        }
        cx.notify();
    }

    pub fn choose_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_path(false, window, cx);
    }
    pub fn choose_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_path(true, window, cx);
    }
    fn choose_path(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(
                if directory {
                    "Choose destination folder"
                } else {
                    "Choose .torrent file"
                }
                .into(),
            ),
        });
        self.choosing_path = true;
        self._prompt = Some(cx.spawn_in(window, async move |view, cx| {
            let result = prompt.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.choosing_path = false;
                match result {
                    Ok(Ok(Some(paths))) if paths.len() == 1 => {
                        let path = paths.into_iter().next().unwrap();
                        if directory {
                            if let Some(source) = &view.source {
                                view.set_destination(
                                    path.join(sanitize_filename(source.name())),
                                    window,
                                    cx,
                                );
                            }
                        } else {
                            view.set_source_path(path, window, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => {
                        // No filesystem paths or tracker details in picker errors.
                        view.validation_requested = true;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn can_submit(&self, cx: &App) -> bool {
        self.submission_issue(cx).is_none()
    }
    fn submission_issue(&self, cx: &App) -> Option<&'static str> {
        if self.choosing_path {
            return Some("Finish choosing a path first.");
        }
        if self.loading {
            return Some("Wait for the torrent file to finish loading.");
        }
        if self.source.is_none() {
            return Some("Choose a valid local .torrent file.");
        }
        let Some(path) = self.destination_path(cx) else {
            return Some("Choose a destination folder or enter the full destination path.");
        };
        if self.target == TargetState::Checking {
            return Some("Wait for the destination check to finish.");
        }
        if self.checked_target.as_ref() != Some(&path) || self.target == TargetState::Unchecked {
            return Some("Edit the destination path or choose a folder to check it.");
        }
        match self.target {
            TargetState::Blocked => {
                return Some(
                    "Choose another destination: symlinks and special files are not supported.",
                );
            }
            TargetState::Error => {
                return Some(
                    "Could not check this destination. Edit its path or choose another folder to retry.",
                );
            }
            TargetState::Exists if self.options.collision != CollisionPolicy::Rename => {
                return Some(
                    "This destination exists. Choose a numbered name or another destination.",
                );
            }
            _ => {}
        }
        if !COLLISIONS.contains(&self.options.collision) {
            return Some("Choose a supported file-conflict option.");
        }
        None
    }

    /// The parent must revalidate this intent before dismissing the confirmation.
    pub fn matches_request(&self, request: &TorrentRequest, cx: &App) -> bool {
        self.can_submit(cx)
            && self
                .source
                .as_ref()
                .is_some_and(|source| source == &request.source)
            && self.destination_path(cx).as_ref() == Some(&request.destination)
            && self.options.collision == request.options.collision
    }
    pub fn submit(&mut self, start_now: bool, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            self.validation_requested = true;
            cx.notify();
            return;
        }
        if let (Some(source), Some(destination)) = (&self.source, self.destination_path(cx)) {
            cx.emit(TorrentRequest {
                source: source.clone(),
                destination,
                start_now,
                options: self.options.clone(),
            });
        }
    }
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if !self.choosing_path {
            cx.emit(DismissTorrent);
        }
    }
    fn set_collision(&mut self, policy: CollisionPolicy, cx: &mut Context<Self>) {
        if !self.choosing_path && COLLISIONS.contains(&policy) {
            self.options.collision = policy;
            cx.notify();
        }
    }
    fn drop_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        if paths.0.len() == 1 {
            self.set_source_path(paths.0[0].clone(), window, cx);
        } else {
            // A rejected drop must not leave a previously valid source submittable.
            self.generation = self.generation.wrapping_add(1);
            self.loader = None;
            self._load_task = None;
            self.source = None;
            self.source_filename = None;
            self.loading = false;
            self.source_error = Some("Drop exactly one local .torrent file.".into());
            self.invalidate_target();
            cx.notify();
        }
    }

    fn metadata(&self, cx: &App) -> AnyElement {
        let Some(source) = &self.source else {
            return v_flex().into_any_element();
        };
        let category = if source.is_multi_file() {
            crate::queue::Category::Other
        } else {
            crate::queue::Category::from_path(Path::new(source.name()))
        };
        let fields = [
            ("Name", source.name().to_owned()),
            (
                "Total size",
                crate::formatting::decimal_bytes(source.total_bytes()),
            ),
            ("Files", source.file_count().to_string()),
            ("Trackers", source.tracker_count().to_string()),
            ("Info hash", source.info_hash()),
        ];
        GroupBox::new()
            .title(Label::new("Torrent contents"))
            .child(
                h_flex()
                    .gap_2()
                    .child(crate::file_icons::category_icon(category, cx))
                    .child(Label::new(category.label())),
            )
            .child(
                DescriptionList::horizontal()
                    .columns(1)
                    .bordered(false)
                    .children(fields.into_iter().map(|(name, value)| {
                        DescriptionItem::new(Label::new(name).into_any_element())
                            .value(Label::new(value).truncate().into_any_element())
                    })),
            )
            .children(source.files().iter().take(FILE_PREVIEW_LIMIT).map(|file| {
                Label::new(format!(
                    "{} — {}",
                    file.path.to_string_lossy(),
                    crate::formatting::decimal_bytes(file.length)
                ))
                .truncate()
            }))
            .when(source.file_count() > FILE_PREVIEW_LIMIT, |group| {
                group.child(Label::new(format!(
                    "{} more files (showing the first {FILE_PREVIEW_LIMIT})",
                    source.file_count() - FILE_PREVIEW_LIMIT
                )))
            })
            .into_any_element()
    }
}

fn is_torrent_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("torrent"))
}
fn sanitized_destination(value: &str) -> Option<PathBuf> {
    if value.trim().is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(sanitize_filename(name)))
}

impl Render for AddTorrent {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
                view.drop_paths(paths, window, cx)
            }))
            .child(
                v_flex()
                    .id("add-torrent-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_3()
                    .child(
                        Form::new()
                            .child(
                                Field::new().label("Torrent file").child(
                                    h_flex()
                                        .gap_2()
                                        .child(Label::new(
                                            self.source_filename
                                                .clone()
                                                .unwrap_or_else(|| "No file selected".into()),
                                        ))
                                        .child(
                                            Button::new("choose-torrent")
                                                .debug_selector(|| "choose-torrent".into())
                                                .label("Choose .torrent file…")
                                                .disabled(self.choosing_path)
                                                .on_click(cx.listener(|view, _, window, cx| {
                                                    view.choose_source(window, cx)
                                                })),
                                        ),
                                ),
                            )
                            .child(
                                Field::new()
                                    .label("Destination (full file or root folder path)")
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                v_flex().flex_1().min_w_0().child(
                                                    Input::new(&self.destination)
                                                        .disabled(self.choosing_path),
                                                ),
                                            )
                                            .child(
                                                Button::new("torrent-destination")
                                                    .debug_selector(|| "torrent-destination".into())
                                                    .label("Choose folder…")
                                                    .disabled(
                                                        self.choosing_path || self.source.is_none(),
                                                    )
                                                    .on_click(cx.listener(
                                                        |view, _, window, cx| {
                                                            view.choose_destination(window, cx)
                                                        },
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                Field::new().label("If the destination exists").child(
                                    RadioGroup::vertical("torrent-collision")
                                        .selected_index(
                                            COLLISIONS.iter().position(|policy| {
                                                *policy == self.options.collision
                                            }),
                                        )
                                        .children(
                                            [
                                                "Reject existing destination",
                                                "Use another name (numbered)",
                                            ]
                                            .into_iter()
                                            .enumerate()
                                            .map(
                                                |(index, label)| {
                                                    Radio::new(("torrent-collision", index))
                                                        .debug_selector(move || {
                                                            format!("torrent-collision-{index}")
                                                        })
                                                        .label(label)
                                                },
                                            ),
                                        )
                                        .disabled(self.choosing_path)
                                        .on_change(cx.listener(|view, index: &usize, _, cx| {
                                            if let Some(policy) = COLLISIONS.get(*index) {
                                                view.set_collision(*policy, cx);
                                            }
                                        })),
                                ),
                            )
                            .child(Field::new().child(Label::new(
                                "Existing files and folders are never overwritten.",
                            ))),
                    )
                    .when(self.loading, |content| {
                        content.child(Label::new("Reading torrent file…"))
                    })
                    .when_some(self.source_error.clone(), |content, error| {
                        content.child(Label::new(error))
                    })
                    .child(self.metadata(cx))
                    .when(self.source.as_ref().is_some_and(|source| source.tracker_count() == 0), |content| {
                        content.child(Label::new("This torrent has no trackers. Peer discovery is unavailable while DHT and local discovery are disabled."))
                    })
                    .when(
                        self.validation_requested || self.target != TargetState::Missing,
                        |content| {
                            content.when_some(self.submission_issue(cx), |content, message| {
                                content.child(Label::new(message))
                            })
                        },
                    ),
            )
            .child(Separator::horizontal())
            .child(Label::new(PRIVACY))
            .child(
                Toolbar::new("add-torrent-actions")
                    .h_auto()
                    .justify_end()
                    .flex_shrink_0()
                    .content(
                        Button::new("close-torrent")
                            .debug_selector(|| "close-torrent".into())
                            .label("Cancel")
                            .disabled(self.choosing_path)
                            .on_click(cx.listener(|view, _, _, cx| view.dismiss(cx))),
                    )
                    .content(
                        Button::new("enqueue-torrent")
                            .debug_selector(|| "enqueue-torrent".into())
                            .label("Download later")
                            .disabled(!self.can_submit(cx))
                            .on_click(cx.listener(|view, _, _, cx| view.submit(false, cx))),
                    )
                    .content(
                        Button::new("start-torrent")
                            .debug_selector(|| "start-torrent".into())
                            .label("Start")
                            .tooltip("Start (Enter)")
                            .disabled(!self.can_submit(cx))
                            .on_click(cx.listener(|view, _, _, cx| view.submit(true, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{AddTorrent, TargetState, TorrentRequest, is_torrent_path, sanitized_destination};
    use gpui_kit::{AppContext, TestAppContext, VisualTestContext, component::Root};
    use std::{
        path::{Path, PathBuf},
        time::Duration,
    };
    use std::{sync::mpsc, time::Instant};
    use ulos_core::transfer::CollisionPolicy;

    // A single-piece, single-file metainfo fixture. No socket is needed to parse it;
    // the intentionally unreachable announce address must never be contacted here.
    fn metainfo() -> Vec<u8> {
        let mut bytes = b"d8:announce27:http://127.0.0.1:1/announce4:infod6:lengthi5e4:name9:hello.bin12:piece lengthi16384e6:pieces20:".to_vec();
        bytes.extend_from_slice(&[0; 20]);
        bytes.extend_from_slice(b"ee");
        bytes
    }

    fn settle(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            if ready(cx) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "torrent draft background observation timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn local_metainfo_load_is_metadata_only() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("local.torrent");
        std::fs::write(&path, metainfo()).unwrap();
        let source = runtime.block_on(ulos_core::torrent::load(path)).unwrap();
        assert_eq!(source.name(), "hello.bin");
        assert_eq!(source.total_bytes(), 5);
        assert_eq!(source.file_count(), 1);
        assert!(!source.is_multi_file());
        assert_eq!(source.tracker_count(), 1);
        assert_eq!(source.files()[0].length, 5);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert!(ulos_core::torrent::parse(b"not metainfo".to_vec()).is_err());
    }

    #[test]
    fn file_choices_and_target_classification() {
        assert!(is_torrent_path(Path::new("local.TORRENT")));
        assert!(!is_torrent_path(Path::new("local.torrent.exe")));
        assert!(sanitized_destination(" ").is_none());
        assert_eq!(
            sanitized_destination("/tmp/bad:name.bin"),
            Some(PathBuf::from("/tmp/bad_name.bin"))
        );
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(TargetState::inspect(directory.path()), TargetState::Exists);
        let file = directory.path().join("file");
        assert_eq!(TargetState::inspect(&file), TargetState::Missing);
        std::fs::write(&file, b"original").unwrap();
        assert_eq!(TargetState::inspect(&file), TargetState::Exists);
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(TargetState::inspect(&link), TargetState::Blocked);
        }
    }

    #[gpui::test]
    fn enter_requires_completed_checks_and_emits_only_an_intent(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.torrent");
        std::fs::write(&path, metainfo()).unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                AddTorrent::new(
                    None,
                    Some(directory.path().to_owned()),
                    runtime.handle().clone(),
                    window,
                    cx,
                )
            });
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        let (sender, requests) = mpsc::channel();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&form, move |_, request: &TorrentRequest, _| {
                sender
                    .send((request.destination.clone(), request.start_now))
                    .unwrap();
            })
        });
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.destination
                    .update(cx, |input, cx| input.focus(window, cx));
                view.submit(true, cx);
                assert!(!view.can_submit(cx));
                view.set_source_path(path, window, cx);
                assert!(view.source.is_none());
                assert!(!view.can_submit(cx));
                view.submit(false, cx);
            })
        });
        assert!(requests.try_recv().is_err());
        settle(cx, |cx| form.read_with(cx, |view, cx| view.can_submit(cx)));
        cx.update(|window, cx| {
            form.read(cx)
                .destination
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            window.draw(cx).clear(cx);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        let target = directory.path().join("hello.bin");
        assert_eq!(requests.try_recv().unwrap(), (target.clone(), true));
        assert!(!target.exists(), "confirmation must not write payloads");
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                let request = TorrentRequest {
                    source: view.source.as_ref().unwrap().clone(),
                    destination: target,
                    start_now: false,
                    options: view.options.clone(),
                };
                assert!(view.matches_request(&request, cx));
                view.choosing_path = true;
                assert!(!view.matches_request(&request, cx));
                view.submit(false, cx);
                view.choosing_path = false;
                let manual = directory.path().join("manual.bin");
                view.set_destination(manual, window, cx);
                assert!(!view.matches_request(&request, cx));
                view.set_source_path(directory.path().join("wrong.txt"), window, cx);
                assert!(view.source.is_none());
                assert!(!view.can_submit(cx));
            })
        });
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn replacement_invalidates_old_source_and_preserves_manual_destination(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let good = directory.path().join("good.torrent");
        let bad = directory.path().join("bad.torrent");
        std::fs::write(&good, metainfo()).unwrap();
        std::fs::write(&bad, b"invalid").unwrap();
        let manual = directory.path().join("manual");
        std::fs::create_dir(&manual).unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                AddTorrent::new(
                    Some(good.clone()),
                    Some(directory.path().to_owned()),
                    runtime.handle().clone(),
                    window,
                    cx,
                )
            });
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_destination(manual.clone(), window, cx);
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, _| {
                view.source.is_some() && view.target == TargetState::Exists
            })
        });
        form.update(cx, |view, cx| {
            assert_eq!(view.destination_path(cx), Some(manual.clone()));
            assert!(!view.can_submit(cx));
            view.set_collision(CollisionPolicy::Rename, cx);
            assert!(view.can_submit(cx));
        });
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_source_path(good.clone(), window, cx);
                view.set_source_path(bad, window, cx);
                assert!(view.source.is_none());
                assert_eq!(view.options.collision, CollisionPolicy::Reject);
            })
        });
        settle(cx, |cx| form.read_with(cx, |view, _| !view.loading));
        form.read_with(cx, |view, cx| {
            assert!(
                view.source.is_none(),
                "stale successful load must not replace invalid draft"
            );
            assert!(view.source_error.is_some());
            assert!(!view.can_submit(cx));
            assert_eq!(view.destination_path(cx), Some(manual));
        });
    }
}
