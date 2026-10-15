use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    component::{
        Disableable, WindowExt,
        button::Button,
        checkbox::Checkbox,
        form::{Field, Form},
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
    naming::{sanitize_filename, suggest_filename},
    transfer::{CollisionPolicy, TransferOptions},
};

pub struct AddRequest {
    pub source: String,
    pub destination: PathBuf,
    pub start_now: bool,
    pub options: TransferOptions,
}

const COLLISION_CHOICES: [CollisionPolicy; 3] = [
    CollisionPolicy::Reject,
    CollisionPolicy::Overwrite,
    CollisionPolicy::Rename,
];

const REPLACEMENT_CONSENT_REQUIRED: &str = "Confirm replacement using the checkbox.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetState {
    Unchecked,
    Checking,
    Missing,
    File,
    Blocked,
    Error,
}
impl TargetState {
    fn inspect(path: &Path) -> Self {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => Self::File,
            Ok(_) => Self::Blocked,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Missing,
            Err(_) => Self::Error,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking destination…",
            Self::Blocked => {
                "Choose another destination: this target is a directory, symlink, or special file."
            }
            Self::Error => {
                "Could not check this destination. Choose another path or edit it to retry."
            }
            _ => "",
        }
    }
}

// A Tokio JoinHandle detaches when dropped. The Add owns cancellation separately
// from GPUI's observer, including when that observer has not been polled yet.
struct PreviewWorker(tokio::task::AbortHandle);
impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub struct DismissAdd;

pub struct AddDownload {
    pub source: Entity<InputState>,
    pub destination: Entity<InputState>,
    pub choosing_path: bool,
    pub(crate) options: TransferOptions,
    pub(crate) overwrite_confirmed: bool,
    suggested_path: Option<String>,
    default_directory: Option<PathBuf>,
    runtime: tokio::runtime::Handle,
    preview_generation: u64,
    preview: PreviewState,
    preview_worker: Option<PreviewWorker>,
    _preview_task: Option<Task<()>>,
    _preview_debounce: Option<Task<()>>,
    target_generation: u64,
    target: TargetState,
    checked_target: Option<PathBuf>,
    _target_task: Option<Task<()>>,
    validation_requested: bool,
    _subscriptions: Vec<Subscription>,
    _prompt: Option<Task<()>>,
}
impl EventEmitter<AddRequest> for AddDownload {}
impl EventEmitter<DismissAdd> for AddDownload {}

impl AddDownload {
    fn category(&self, cx: &App) -> crate::queue::Category {
        let path = self.destination_path(cx).unwrap_or_else(|| {
            PathBuf::from(suggest_filename(self.source.read(cx).value().trim()).unwrap_or_default())
        });
        crate::queue::Category::from_path(&path)
    }
    fn destination_path(&self, cx: &App) -> Option<PathBuf> {
        let value = self.destination.read(cx).value();
        if value.trim().is_empty() {
            return None;
        }
        let raw = PathBuf::from(value.as_ref());
        let name = raw.file_name()?.to_str()?;
        Some(raw.with_file_name(sanitize_filename(name)))
    }

    pub(crate) fn has_file_target(&self, cx: &App) -> bool {
        self.target == TargetState::File
            && self.checked_target.is_some()
            && self.checked_target == self.destination_path(cx)
    }

    fn check_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overwrite_confirmed = false;
        self.options.collision = CollisionPolicy::Reject;
        self.target_generation = self.target_generation.wrapping_add(1);
        self._target_task = None;
        self.target = TargetState::Unchecked;
        self.checked_target = None;
        let Some(path) = self.destination_path(cx) else {
            return;
        };
        self.target = TargetState::Checking;
        let generation = self.target_generation;
        let runtime = self.runtime.clone();
        self._target_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let expected = path.clone();
            let worker = runtime.spawn_blocking(move || TargetState::inspect(&path));
            while !worker.is_finished() {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
            }
            let state = worker.await.unwrap_or(TargetState::Error);
            let _ = view.update_in(cx, |view, _, cx| {
                if generation == view.target_generation
                    && view.destination_path(cx).as_ref() == Some(&expected)
                {
                    view.target = state;
                    view.checked_target = Some(expected);
                    cx.notify();
                }
            });
        }));
    }

    fn schedule_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_preview();
        if suggest_filename(self.source.read(cx).value().trim()).is_err() {
            return;
        }
        self.preview = PreviewState::Loading;
        self._preview_debounce = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(450))
                .await;
            let _ = view.update_in(cx, |view, window, cx| view.refresh_preview(window, cx));
        }));
    }
    pub fn new(
        default_directory: Option<PathBuf>,
        runtime: tokio::runtime::Handle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let source =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://example.org/file.zip"));
        let destination =
            cx.new(|cx| InputState::new(window, cx).placeholder("Full destination path"));
        let source_sub = cx.subscribe_in(&source, window, |view, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                view.detect_name(window, cx);
                view.schedule_preview(window, cx);
                view.check_target(window, cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                view.submit_from_input(window, cx);
            }
            cx.notify();
        });
        let mut subscriptions = vec![source_sub];
        subscriptions.push(
            cx.subscribe_in(&destination, window, |view, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.overwrite_confirmed = false;
                    view.suggested_path = None;
                    view.check_target(window, cx);
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.submit_from_input(window, cx);
                }
                cx.notify();
            }),
        );
        Self {
            source,
            destination,
            choosing_path: false,
            options: TransferOptions {
                connections: 4,
                ..TransferOptions::default()
            },
            overwrite_confirmed: false,
            suggested_path: None,
            default_directory,
            runtime,
            preview_generation: 0,
            preview: PreviewState::NotRequested,
            preview_worker: None,
            _preview_task: None,
            _preview_debounce: None,
            target_generation: 0,
            target: TargetState::Unchecked,
            checked_target: None,
            _target_task: None,
            validation_requested: false,
            _subscriptions: subscriptions,
            _prompt: None,
        }
    }

    pub(crate) fn set_connections(&mut self, count: usize, cx: &mut Context<Self>) -> bool {
        if self.choosing_path || !CONNECTION_CHOICES.contains(&count) {
            return false;
        }
        self.options.connections = count;
        cx.notify();
        true
    }

    pub(crate) fn set_collision(&mut self, policy: CollisionPolicy, cx: &mut Context<Self>) {
        if self.choosing_path || (policy != CollisionPolicy::Reject && !self.has_file_target(cx)) {
            return;
        }
        if self.options.collision != policy {
            self.overwrite_confirmed = false;
        }
        self.options.collision = policy;
        cx.notify();
    }

    pub(crate) fn authorize_overwrite(&mut self, authorized: bool, cx: &mut Context<Self>) {
        self.overwrite_confirmed = authorized
            && self.has_file_target(cx)
            && self.options.collision == CollisionPolicy::Overwrite;
        cx.notify();
    }

    fn collision_label(policy: CollisionPolicy) -> &'static str {
        match policy {
            CollisionPolicy::Reject => "Do not overwrite",
            CollisionPolicy::Overwrite => "Override existing file",
            CollisionPolicy::Rename => "Add another file (numbered)",
        }
    }

    fn connections_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        RadioGroup::horizontal("add-connections")
            .selected_index(
                CONNECTION_CHOICES
                    .iter()
                    .position(|count| *count == self.options.connections),
            )
            .children(CONNECTION_CHOICES.into_iter().map(|count| {
                Radio::new(("connections", count as u64))
                    .debug_selector(move || format!("connections-{count}"))
                    .label(count.to_string())
                    .accessibility_label(format!("{count} connections"))
            }))
            .disabled(self.choosing_path)
            .on_change(cx.listener(|view, index: &usize, _, cx| {
                if let Some(count) = CONNECTION_CHOICES.get(*index) {
                    view.set_connections(*count, cx);
                }
            }))
    }

    fn collision_control(&self, cx: &mut Context<Self>) -> impl IntoElement {
        RadioGroup::vertical("add-file-conflict")
            .selected_index(
                COLLISION_CHOICES
                    .iter()
                    .position(|policy| *policy == self.options.collision),
            )
            .children(
                COLLISION_CHOICES
                    .into_iter()
                    .enumerate()
                    .map(|(index, policy)| {
                        Radio::new(("collision", index))
                            .debug_selector(move || format!("collision-{index}"))
                            .label(Self::collision_label(policy))
                    }),
            )
            .disabled(self.choosing_path)
            .on_change(cx.listener(|view, index: &usize, _, cx| {
                if let Some(policy) = COLLISION_CHOICES.get(*index) {
                    view.set_collision(*policy, cx);
                }
            }))
    }

    pub fn prefill(&mut self, source: String, window: &mut Window, cx: &mut Context<Self>) {
        self.invalidate_preview();
        self.source
            .update(cx, |input, cx| input.set_value(source, window, cx));
        self.detect_name(window, cx);
        self.check_target(window, cx);
    }

    fn detect_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.read(cx).value();
        let Ok(name) = suggest_filename(source.trim()) else {
            return;
        };
        let current = self.destination.read(cx).value().to_string();
        if current.trim().is_empty() || self.suggested_path.as_deref() == Some(&current) {
            let Some(directory) = self.default_directory.as_deref() else {
                return;
            };
            let path = directory.join(name);
            if let Some(path) = path.to_str() {
                self.overwrite_confirmed = false;
                self.destination
                    .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx));
                self.check_target(window, cx);
                self.suggested_path = Some(path.to_owned());
            }
        }
    }

    pub(crate) fn invalidate_preview(&mut self) {
        self.preview_generation = self.preview_generation.wrapping_add(1);
        self.preview_worker = None;
        self._preview_task = None;
        self._preview_debounce = None;
        self.preview = PreviewState::NotRequested;
    }

    pub(crate) fn refresh_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.read(cx).value().trim().to_owned();
        if suggest_filename(&source).is_err() {
            return;
        }
        self.preview_generation = self.preview_generation.wrapping_add(1);
        let generation = self.preview_generation;
        self.preview = PreviewState::Loading;
        let expected_source = source.clone();
        // Tokio owns HTTP work; replacing the draft aborts the previous request.
        let worker = self
            .runtime
            .spawn(async move { ulos_core::preview::inspect(&source).await });
        self.preview_worker = Some(PreviewWorker(worker.abort_handle()));
        self._preview_task = Some(cx.spawn_in(window, async move |view, cx| {
            while !worker.is_finished() {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
            }
            let result = worker.await;
            let _ = view.update_in(cx, |view, _, cx| {
                if view.source.read(cx).value().trim() != expected_source {
                    return;
                }
                let state = match result {
                    Ok(Ok(metadata)) => PreviewState::Ready(metadata.size),
                    // Avoid retaining/displaying request details or sensitive URLs.
                    _ => PreviewState::Error,
                };
                view.apply_preview(generation, state);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_preview(&mut self, generation: u64, state: PreviewState) {
        if generation == self.preview_generation {
            self.preview = state;
            self.preview_worker = None;
        }
    }

    pub fn can_submit(&self, cx: &App) -> bool {
        self.submission_issue(cx).is_none()
    }

    fn submission_issue(&self, cx: &App) -> Option<&'static str> {
        if self.choosing_path {
            return Some("Finish choosing a destination first.");
        }
        if self.source.read(cx).value().trim().is_empty() {
            return Some("Enter a download address.");
        }
        if suggest_filename(self.source.read(cx).value().trim()).is_err() {
            return Some("Enter a valid HTTP or HTTPS address without embedded credentials.");
        }
        let Some(path) = self.destination_path(cx) else {
            return Some("Choose a destination file with Save As or enter its full path.");
        };
        if self.target == TargetState::Checking {
            return Some("Wait for the destination check to finish.");
        }
        if self.checked_target.as_ref() != Some(&path) || self.target == TargetState::Unchecked {
            return Some("Choose a destination file with Save As or edit its path to check it.");
        }
        if matches!(self.target, TargetState::Blocked | TargetState::Error) {
            return Some(self.target.label());
        }
        if self.target == TargetState::File && self.options.collision == CollisionPolicy::Reject {
            return Some(
                "This file already exists. Choose another destination, Override existing file, or Add another file (numbered).",
            );
        }
        if self.options.collision != CollisionPolicy::Reject && !self.has_file_target(cx) {
            return Some(
                "Choose a destination file again before selecting a file-conflict option.",
            );
        }
        if self.options.collision == CollisionPolicy::Overwrite && !self.overwrite_confirmed {
            return Some(REPLACEMENT_CONSENT_REQUIRED);
        }
        None
    }

    fn submission_guidance(&self, cx: &App) -> Option<&'static str> {
        match self.submission_issue(cx) {
            Some(REPLACEMENT_CONSENT_REQUIRED) => None,
            issue => issue,
        }
    }

    fn submit_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(true, cx);
        if !self.can_submit(cx) && !self.choosing_path {
            if suggest_filename(self.source.read(cx).value().trim()).is_err() {
                self.source.update(cx, |input, cx| input.focus(window, cx));
            } else if self.destination_path(cx).is_none() {
                self.destination
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
    }

    pub fn submit(&mut self, start_now: bool, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            self.validation_requested = true;
            cx.notify();
            return;
        }
        let Some(destination) = self.destination_path(cx) else {
            return;
        };
        cx.emit(AddRequest {
            source: self.source.read(cx).value().trim().to_owned(),
            destination,
            start_now,
            options: self.options.clone(),
        });
    }

    pub fn choose_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        self.detect_name(window, cx);
        let path = PathBuf::from(self.destination.read(cx).value().as_ref());
        let suggested = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_filename)
            .unwrap_or_else(|| "download.bin".to_owned());
        let directory = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| self.default_directory.as_deref().unwrap_or(Path::new(".")));
        let prompt = cx.prompt_for_new_path(directory, Some(&suggested));
        self.choosing_path = true;
        self._prompt = Some(cx.spawn_in(window, async move |view, cx| {
            let result = prompt.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.choosing_path = false;
                match result {
                    Ok(Ok(Some(path))) => view.set_path(path, window, cx),
                    Ok(Ok(None)) => {}
                    _ => window.push_notification(
                        "Could not open the save dialog; enter a path manually",
                        cx,
                    ),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.overwrite_confirmed = false;
        match path.to_str() {
            Some(path) => {
                // Picker selection is a manual choice, including when it happens
                // to equal the previous suggestion.
                self.suggested_path = None;
                self.destination
                    .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx));
                self.check_target(window, cx);
            }
            None => window
                .push_notification("This filename cannot be represented in the path input", cx),
        }
    }
}

impl Render for AddDownload {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                v_flex()
                    .id("add-download-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        Form::new()
                            .child(
                                Field::new()
                                    .label("Address")
                                    .child(Input::new(&self.source).disabled(self.choosing_path)),
                            )
                            .child(
                                Field::new().label("Save As").child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            v_flex().flex_1().min_w_0().child(
                                                Input::new(&self.destination)
                                                    .disabled(self.choosing_path),
                                            ),
                                        )
                                        .child(
                                            Button::new("choose-destination")
                                                .label("Save As…")
                                                .disabled(self.choosing_path)
                                                .on_click(cx.listener(|view, _, window, cx| {
                                                    view.choose_path(window, cx)
                                                })),
                                        ),
                                ),
                            )
                            .child(
                                Field::new().label("Size").child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(crate::file_icons::category_icon(
                                            self.category(cx),
                                            cx,
                                        ))
                                        .child(Label::new(self.preview.label()))
                                        .child(Label::new(self.category(cx).label()))
                                        .when(
                                            matches!(
                                                self.preview,
                                                PreviewState::Error | PreviewState::NotRequested
                                            ) && !self.source.read(cx).value().trim().is_empty(),
                                            |row| {
                                                row.child(
                                                    Button::new("preview-size")
                                                        .label("Check size")
                                                        .disabled(self.choosing_path)
                                                        .on_click(cx.listener(
                                                            |view, _, window, cx| {
                                                                view.refresh_preview(window, cx)
                                                            },
                                                        )),
                                                )
                                            },
                                        ),
                                ),
                            )
                            .child(
                                Field::new()
                                    .label("Connections")
                                    .child(self.connections_control(cx)),
                            )
                            .when(self.has_file_target(cx), |form| {
                                form.child(
                                    Field::new()
                                        .label("File already exists")
                                        .child(self.collision_control(cx)),
                                )
                            })
                            .when(
                                matches!(
                                    self.target,
                                    TargetState::Checking
                                        | TargetState::Blocked
                                        | TargetState::Error
                                ),
                                |form| {
                                    form.child(Field::new().child(Label::new(self.target.label())))
                                },
                            )
                            .when(
                                self.options.collision == CollisionPolicy::Overwrite,
                                |form| {
                                    form.child(
                                        Field::new().child(
                                            Checkbox::new("authorize-overwrite")
                                                .label(
                                                    "I authorize replacing this destination file",
                                                )
                                                .checked(self.overwrite_confirmed)
                                                .disabled(self.choosing_path)
                                                .on_click(cx.listener(|view, checked, _, cx| {
                                                    view.authorize_overwrite(*checked, cx)
                                                })),
                                        ),
                                    )
                                },
                            ),
                    )
                    .when(
                        self.validation_requested || self.has_file_target(cx),
                        |content| {
                            content.when_some(self.submission_guidance(cx), |content, message| {
                                content.child(Label::new(message))
                            })
                        },
                    ),
            )
            .child(Separator::horizontal())
            .child(
                Toolbar::new("add-download-actions")
                    .h_auto()
                    .justify_end()
                    .flex_shrink_0()
                    .content(
                        Button::new("close-add")
                            .debug_selector(|| "close-add".into())
                            .label("Cancel")
                            .disabled(self.choosing_path)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissAdd))),
                    )
                    .content(
                        Button::new("enqueue-download")
                            .debug_selector(|| "enqueue-download".into())
                            .label("Download later")
                            .disabled(!self.can_submit(cx))
                            .on_click(cx.listener(|view, _, _, cx| view.submit(false, cx))),
                    )
                    .content(
                        Button::new("download-now")
                            .debug_selector(|| "download-now".into())
                            .label("Start download")
                            .tooltip("Start download (Enter)")
                            .disabled(!self.can_submit(cx))
                            .on_click(cx.listener(|view, _, _, cx| view.submit(true, cx))),
                    ),
            )
    }
}

const CONNECTION_CHOICES: [usize; 5] = [1, 2, 4, 8, 16];

#[derive(Debug, PartialEq, Eq)]
enum PreviewState {
    NotRequested,
    Loading,
    Ready(Option<u64>),
    Error,
}
impl PreviewState {
    fn label(&self) -> String {
        match self {
            Self::NotRequested => "Unknown — not checked".into(),
            Self::Loading => "Checking…".into(),
            Self::Ready(Some(bytes)) => crate::formatting::decimal_bytes(*bytes),
            Self::Ready(None) => "Unknown — HEAD supplied no usable size".into(),
            Self::Error => "Unknown — size check failed".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AddDownload;
    use gpui_kit::{AppContext, TestAppContext, component::Root};
    use std::{
        path::PathBuf,
        sync::mpsc,
        time::{Duration, Instant},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    use ulos_core::transfer::CollisionPolicy;

    #[gpui::test]
    fn enter_validates_fields_then_starts_a_complete_form(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        let (sender, requests) = mpsc::channel();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&form, move |_, request: &super::AddRequest, _| {
                sender
                    .send((
                        request.destination.clone(),
                        request.start_now,
                        request.options.collision,
                    ))
                    .unwrap();
            })
        });
        cx.update(|window, cx| {
            form.read(cx)
                .source
                .clone()
                .update(cx, |input, cx| input.focus(window, cx));
            window.draw(cx).clear(cx);
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        form.read_with(cx, |view, cx| {
            assert!(view.validation_requested);
            assert_eq!(view.submission_issue(cx), Some("Enter a download address."));
        });
        assert!(requests.try_recv().is_err());
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill("https://example.org/file.zip".into(), window, cx)
            })
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        form.read_with(cx, |view, cx| {
            assert!(
                view.submission_issue(cx)
                    .unwrap()
                    .starts_with("Choose a destination")
            )
        });
        assert!(requests.try_recv().is_err());
        let destination = directory.path().join("file.zip");
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_path(destination.clone(), window, cx);
                assert!(!view.can_submit(cx));
            })
        });
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
        assert_eq!(
            requests.try_recv().unwrap(),
            (destination.clone(), true, CollisionPolicy::Reject)
        );
        assert!(requests.try_recv().is_err());
        assert!(!destination.exists(), "form events must not create files");
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill("https://user:SECRET@example.org/file".into(), window, cx)
            })
        });
        form.update(cx, |view, cx| {
            view.submit(true, cx);
            assert!(
                view.submission_issue(cx)
                    .unwrap()
                    .starts_with("Enter a valid HTTP")
            );
        });
        assert!(requests.try_recv().is_err());
    }

    #[gpui::test]
    fn existing_reject_disables_download_actions_but_cancel_still_works(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing.zip");
        std::fs::write(&destination, b"original").unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        let (sender, events) = mpsc::channel();
        let cancel_sender = sender.clone();
        let _requests = cx.update(|_, cx| {
            cx.subscribe(&form, move |_, _: &super::AddRequest, _| {
                sender.send("request").unwrap();
            })
        });
        let _dismiss = cx.update(|_, cx| {
            cx.subscribe(&form, move |_, _: &super::DismissAdd, _| {
                cancel_sender.send("cancel").unwrap();
            })
        });
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill("https://example.org/file.zip".into(), window, cx);
                view.set_path(destination.clone(), window, cx);
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, cx| view.has_file_target(cx))
        });
        form.update(cx, |view, cx| {
            assert!(!view.can_submit(cx));
            assert!(
                view.submission_issue(cx)
                    .unwrap()
                    .starts_with("This file already exists")
            );
            view.submit(true, cx);
            view.submit(false, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for id in ["download-now", "enqueue-download"] {
            let bounds = cx.debug_bounds(id).unwrap();
            cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
            cx.run_until_parked();
        }
        assert!(events.try_recv().is_err());
        form.update(cx, |view, cx| {
            view.set_collision(CollisionPolicy::Rename, cx);
            assert!(view.can_submit(cx));
            view.set_collision(CollisionPolicy::Reject, cx);
            assert!(!view.can_submit(cx));
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let cancel = cx.debug_bounds("close-add").unwrap();
        cx.simulate_click(cancel.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        assert_eq!(events.try_recv().unwrap(), "cancel");
        assert_eq!(std::fs::read(destination).unwrap(), b"original");
    }

    // Retain each accepted socket without replying, so the real HTTP worker is
    // demonstrably pending when the UI edits, replaces or destroys its owner.
    struct PreviewServer {
        source: String,
        requests: mpsc::Receiver<(String, TcpStream)>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for PreviewServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl PreviewServer {
        fn new(runtime: &tokio::runtime::Runtime) -> Self {
            let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
            let source = format!(
                "http://{}/file?private=SECRET",
                listener.local_addr().unwrap()
            );
            let (sender, requests) = mpsc::channel();
            let task = runtime.spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        if stream.read_exact(&mut byte).await.is_err() {
                            break;
                        }
                        request.push(byte[0]);
                        assert!(request.len() < 16_384);
                    }
                    if sender
                        .send((String::from_utf8(request).unwrap(), stream))
                        .is_err()
                    {
                        break;
                    }
                }
            });
            Self {
                source,
                requests,
                task,
            }
        }
        fn request(&self) -> TcpStream {
            let (request, stream) = self.requests.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(request.starts_with("HEAD /file?private=SECRET HTTP/1.1\r\n"));
            stream
        }
    }

    fn assert_preview_socket_closed(runtime: &tokio::runtime::Runtime, mut stream: TcpStream) {
        runtime.block_on(async {
            let mut byte = [0];
            let result = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut byte))
                .await
                .expect("aborted preview must close its pending request");
            assert!(matches!(result, Ok(0) | Err(_)));
        });
    }

    #[gpui::test]
    fn explicit_preview_refresh_owns_io_and_cancels_on_edit_replacement_and_drop(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = PreviewServer::new(&runtime);
        cx.update(gpui_kit::init);
        let mut form = None;
        let (root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill(server.source.clone(), window, cx);
                view.set_path(PathBuf::from("/tmp/manual-preview.zip"), window, cx);
            })
        });
        cx.run_until_parked();
        assert!(matches!(
            server.requests.recv_timeout(Duration::from_millis(40)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        cx.update(|window, cx| form.update(cx, |view, cx| view.refresh_preview(window, cx)));
        let first = server.request();
        let old = form.read_with(cx, |view, _| view.preview_generation);
        // An explicit new request replaces even an outstanding request.
        cx.update(|window, cx| form.update(cx, |view, cx| view.refresh_preview(window, cx)));
        let mut second = server.request();
        assert_preview_socket_closed(&runtime, first);
        runtime
            .block_on(second.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 42\r\n\r\n"))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(25));
            cx.run_until_parked();
            if form.read_with(cx, |view, _| {
                view.preview == super::PreviewState::Ready(Some(42))
            }) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "preview response was not observed"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        form.update(cx, |view, cx| {
            view.apply_preview(old, super::PreviewState::Ready(Some(999)));
            assert_eq!(view.preview, super::PreviewState::Ready(Some(42)));
            assert_eq!(
                view.destination.read(cx).value().as_ref(),
                "/tmp/manual-preview.zip"
            );
        });
        cx.update(|window, cx| form.update(cx, |view, cx| view.refresh_preview(window, cx)));
        let third = server.request();
        // Exercise the actual InputEvent::Change subscription, not just prefill.
        cx.update(|window, cx| {
            form.read(cx).source.clone().update(cx, |input, cx| {
                input.set_value(format!("{}/edited", server.source), window, cx);
                cx.emit(gpui_kit::component::input::InputEvent::Change);
            })
        });
        cx.run_until_parked();
        assert_preview_socket_closed(&runtime, third);
        form.read_with(cx, |view, _| {
            assert_eq!(view.preview, super::PreviewState::Loading)
        });
        assert!(matches!(
            server.requests.recv_timeout(Duration::from_millis(40)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill(server.source.clone(), window, cx);
                view.refresh_preview(window, cx);
            })
        });
        let fourth = server.request();
        form.update(cx, |view, _| view.invalidate_preview());
        cx.update(|window, _| window.remove_window());
        drop(form);
        drop(root);
        cx.run_until_parked();
        assert_preview_socket_closed(&runtime, fourth);
    }

    #[gpui::test]
    fn defaults_and_connection_choices_are_real_options(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        form.update(cx, |view, cx| {
            assert_eq!(view.options.connections, 4);
            assert_eq!(view.options.bytes_per_second, None);
            assert_eq!(view.options.collision, CollisionPolicy::Reject);
            assert!(!view.overwrite_confirmed);
            for count in [1, 2, 4, 8, 16] {
                assert!(view.set_connections(count, cx));
                assert_eq!(view.options.connections, count);
            }
            for count in [0, 3, 9, 17, usize::MAX] {
                assert!(!view.set_connections(count, cx));
                assert_eq!(view.options.connections, 16);
            }
        });
    }

    fn settle(
        cx: &mut gpui_kit::VisualTestContext,
        mut ready: impl FnMut(&mut gpui_kit::VisualTestContext) -> bool,
    ) {
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
                "Add background observation timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[gpui::test]
    fn overwrite_requires_an_existing_sanitized_destination_and_fresh_consent(
        cx: &mut TestAppContext,
    ) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("file_.zip");
        std::fs::write(&existing, b"keep").unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.prefill("https://example.org/file.zip".into(), window, cx);
                view.set_path(directory.path().join("file?.zip"), window, cx);
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, _| view.target == super::TargetState::File)
        });
        form.update(cx, |view, cx| {
            assert!(view.has_file_target(cx));
            assert_eq!(view.checked_target.as_ref(), Some(&existing));
            view.set_collision(CollisionPolicy::Overwrite, cx);
            assert!(!view.can_submit(cx));
            assert_eq!(view.submission_guidance(cx), None);
            assert_eq!(
                view.submission_issue(cx),
                Some(super::REPLACEMENT_CONSENT_REQUIRED)
            );
            view.submit(true, cx);
            assert!(!view.can_submit(cx));
            assert_eq!(view.submission_guidance(cx), None);
            view.authorize_overwrite(true, cx);
            assert!(view.can_submit(cx));
        });
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_path(directory.path().join("missing.zip"), window, cx)
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, _| view.target == super::TargetState::Missing)
        });
        form.update(cx, |view, cx| {
            assert!(!view.has_file_target(cx));
            assert!(!view.overwrite_confirmed);
            assert_eq!(view.options.collision, CollisionPolicy::Reject);
            view.set_collision(CollisionPolicy::Overwrite, cx);
            view.authorize_overwrite(true, cx);
            assert_eq!(view.options.collision, CollisionPolicy::Reject);
            assert!(!view.overwrite_confirmed);
            assert!(view.can_submit(cx));
        });
        // Old async file metadata cannot undo a newer directory selection.
        let blocked = directory.path().join("folder");
        std::fs::create_dir(&blocked).unwrap();
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_path(existing.clone(), window, cx);
                view.set_path(blocked.clone(), window, cx);
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, _| view.target == super::TargetState::Blocked)
        });
        form.read_with(cx, |view, cx| assert!(!view.can_submit(cx)));
        assert_eq!(std::fs::read(existing).unwrap(), b"keep");
    }

    #[gpui::test]
    fn address_edits_debounce_head_and_ignore_stale_results(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let server = PreviewServer::new(&runtime);
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        let edit = |value: String, cx: &mut gpui_kit::VisualTestContext| {
            cx.update(|window, cx| {
                form.read(cx).source.clone().update(cx, |input, cx| {
                    input.set_value(value, window, cx);
                    cx.emit(gpui_kit::component::input::InputEvent::Change);
                });
            })
        };
        edit(server.source.clone(), cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        assert!(server.requests.try_recv().is_err());
        edit(server.source.clone(), cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
        cx.run_until_parked();
        assert!(server.requests.try_recv().is_err());
        cx.executor().advance_clock(Duration::from_millis(151));
        cx.run_until_parked();
        let mut stream = server.request();
        runtime
            .block_on(stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1500000\r\n\r\n"))
            .unwrap();
        settle(cx, |cx| {
            form.read_with(cx, |view, _| {
                view.preview == super::PreviewState::Ready(Some(1_500_000))
            })
        });
        form.read_with(cx, |view, _| assert_eq!(view.preview.label(), "1.5 MB"));
        edit(server.source.clone(), cx);
        settle(cx, |cx| {
            form.read_with(cx, |view, _| view.preview_worker.is_some())
        });
        let pending = server.request();
        let old = form.read_with(cx, |view, _| view.preview_generation);
        edit("file:///not-http".into(), cx);
        cx.run_until_parked();
        assert_preview_socket_closed(&runtime, pending);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        form.update(cx, |view, _| {
            view.apply_preview(old, super::PreviewState::Ready(Some(99)));
            assert_eq!(view.preview, super::PreviewState::NotRequested);
        });
        assert!(server.requests.try_recv().is_err());
    }

    #[gpui::test]
    fn stock_radios_change_connections_and_conditional_conflicts(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.zip");
        std::fs::write(&path, b"keep").unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        let form = form.unwrap();
        cx.update(|window, cx| {
            form.update(cx, |view, cx| view.set_path(path, window, cx));
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("collision-1").is_none());
        settle(cx, |cx| {
            form.read_with(cx, |view, cx| view.has_file_target(cx))
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx
            .debug_bounds("connections-8")
            .expect("stock connection radio");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        form.read_with(cx, |view, _| assert_eq!(view.options.connections, 8));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx
            .debug_bounds("collision-1")
            .expect("existing-file conflict radio");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        form.read_with(cx, |view, _| {
            assert_eq!(view.options.collision, CollisionPolicy::Overwrite);
            assert!(!view.overwrite_confirmed);
        });
        cx.update(|window, cx| {
            form.update(cx, |view, cx| {
                view.set_path(directory.path().join("new.zip"), window, cx)
            })
        });
        settle(cx, |cx| {
            form.read_with(cx, |view, _| view.target == super::TargetState::Missing)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("collision-1").is_none());
    }

    #[test]
    fn target_check_refuses_directories_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(
            super::TargetState::inspect(directory.path()),
            super::TargetState::Blocked
        );
        assert_eq!(
            super::TargetState::inspect(&directory.path().join("missing")),
            super::TargetState::Missing
        );
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(directory.path().join("missing"), &link).unwrap();
            assert_eq!(
                super::TargetState::inspect(&link),
                super::TargetState::Blocked
            );
        }
    }

    #[gpui::test]
    fn downloads_default_and_manual_destination_survive_url_changes(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                AddDownload::new(
                    Some(PathBuf::from("/tmp/Downloads")),
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
                view.prefill("https://example.org/first.zip".into(), window, cx);
                assert_eq!(
                    view.destination.read(cx).value().as_ref(),
                    "/tmp/Downloads/first.zip"
                );
                view.prefill("https://example.org/second.zip".into(), window, cx);
                assert_eq!(
                    view.destination.read(cx).value().as_ref(),
                    "/tmp/Downloads/second.zip"
                );
                view.destination.update(cx, |input, cx| {
                    input.set_value("/tmp/manual.zip", window, cx)
                });
                view.prefill("https://example.org/third.zip".into(), window, cx);
                assert_eq!(
                    view.destination.read(cx).value().as_ref(),
                    "/tmp/manual.zip"
                );
                // Clipboard/programmatic prefill remains network-free.
                assert!(view._preview_task.is_none());
                assert_eq!(view.preview, super::PreviewState::NotRequested);
                view.set_path(PathBuf::from("/tmp/picker.zip"), window, cx);
                view.prefill("https://example.org/fourth.zip".into(), window, cx);
                assert_eq!(
                    view.destination.read(cx).value().as_ref(),
                    "/tmp/picker.zip"
                );
            })
        });
    }

    #[gpui::test]
    fn stale_preview_replies_cannot_replace_current_metadata(cx: &mut TestAppContext) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        cx.update(gpui_kit::init);
        let mut form = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| AddDownload::new(None, runtime.handle().clone(), window, cx));
            form = Some(view.clone());
            Root::new(view, window, cx)
        });
        form.unwrap().update(cx, |view, _| {
            let old = view.preview_generation;
            view.invalidate_preview();
            view.apply_preview(old, super::PreviewState::Ready(Some(99)));
            assert_eq!(view.preview, super::PreviewState::NotRequested);
            view.apply_preview(view.preview_generation, super::PreviewState::Ready(Some(0)));
            assert_eq!(view.preview.label(), "0 B");
            view.apply_preview(old, super::PreviewState::Error);
            assert_eq!(view.preview, super::PreviewState::Ready(Some(0)));
        });
    }

    #[test]
    fn preview_unknown_and_failure_are_not_zero_or_resume_claims() {
        assert!(
            super::PreviewState::Ready(None)
                .label()
                .starts_with("Unknown")
        );
        assert!(super::PreviewState::Error.label().starts_with("Unknown"));
        assert!(
            super::PreviewState::NotRequested
                .label()
                .contains("not checked")
        );
    }
}
