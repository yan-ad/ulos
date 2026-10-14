use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, WindowExt,
        button::{Button, ButtonVariants},
        form::{Field, Form},
        input::{Input, InputEvent, InputState},
    },
    *,
};
use ulos_core::naming::{sanitize_filename, suggest_filename};

pub struct AddRequest {
    pub source: String,
    pub destination: PathBuf,
    pub start_now: bool,
}
pub struct DismissAdd;

pub struct AddDownload {
    pub source: Entity<InputState>,
    pub destination: Entity<InputState>,
    pub filename: Entity<InputState>,
    pub choosing_path: bool,
    suggestion: String,
    selected_folder: Option<PathBuf>,
    _subscriptions: Vec<Subscription>,
    _prompt: Option<Task<()>>,
}
impl EventEmitter<AddRequest> for AddDownload {}
impl EventEmitter<DismissAdd> for AddDownload {}

impl AddDownload {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://example.org/file.zip"));
        let destination = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Select a folder or enter a full save path")
        });
        let filename =
            cx.new(|cx| InputState::new(window, cx).placeholder("Detected from the URL"));
        let source_sub = cx.subscribe_in(&source, window, |view, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                view.detect_name(window, cx);
            }
            if matches!(event, InputEvent::PressEnter { .. }) {
                view.submit(true, cx);
            }
            cx.notify();
        });
        let mut subscriptions = vec![source_sub];
        subscriptions.push(cx.subscribe_in(
            &destination,
            window,
            |view, input, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.selected_folder = None;
                    let path = PathBuf::from(input.read(cx).value().as_ref());
                    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                        view.filename
                            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
                    }
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.submit(true, cx);
                }
                cx.notify();
            },
        ));
        subscriptions.push(
            cx.subscribe_in(&filename, window, |view, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    view.sync_name(window, cx);
                }
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.submit(true, cx);
                }
                cx.notify();
            }),
        );
        Self {
            source,
            destination,
            filename,
            choosing_path: false,
            suggestion: String::new(),
            selected_folder: None,
            _subscriptions: subscriptions,
            _prompt: None,
        }
    }

    pub fn prefill(&mut self, source: String, window: &mut Window, cx: &mut Context<Self>) {
        self.source
            .update(cx, |input, cx| input.set_value(source, window, cx));
        self.detect_name(window, cx);
    }

    fn detect_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.read(cx).value();
        if let Ok(next) = suggest_filename(source.trim()) {
            let current = self.filename.read(cx).value();
            if current.trim().is_empty() || current.as_ref() == self.suggestion {
                self.filename
                    .update(cx, |input, cx| input.set_value(next.clone(), window, cx));
                self.sync_name(window, cx);
            }
            self.suggestion = next;
        }
    }

    fn sync_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = PathBuf::from(self.destination.read(cx).value().as_ref());
        if current.as_os_str().is_empty() {
            return;
        }
        let name = sanitize_filename(&self.filename.read(cx).value());
        let path = if let Some(folder) = &self.selected_folder {
            folder.join(name)
        } else {
            current.with_file_name(name)
        };
        if let Some(path) = path.to_str() {
            self.destination
                .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx));
        }
    }

    pub fn can_submit(&self, cx: &App) -> bool {
        !self.choosing_path
            && !self.source.read(cx).value().trim().is_empty()
            && !self.destination.read(cx).value().trim().is_empty()
    }

    pub fn submit(&mut self, start_now: bool, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return;
        }
        let raw = if let Some(folder) = &self.selected_folder {
            folder.join(sanitize_filename(&self.filename.read(cx).value()))
        } else {
            PathBuf::from(self.destination.read(cx).value().as_ref())
        };
        let Some(name) = raw.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let destination = raw.with_file_name(sanitize_filename(name));
        cx.emit(AddRequest {
            source: self.source.read(cx).value().trim().to_owned(),
            destination,
            start_now,
        });
    }

    pub fn choose_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        self.detect_name(window, cx);
        let raw_name = self.filename.read(cx).value();
        let suggested = if raw_name.trim().is_empty() {
            "download.bin".to_owned()
        } else {
            sanitize_filename(&raw_name)
        };
        let path = PathBuf::from(self.destination.read(cx).value().as_ref());
        let directory = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
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

    pub(crate) fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.choosing_path {
            return;
        }
        self.detect_name(window, cx);
        let name = sanitize_filename(&self.filename.read(cx).value());
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select download folder".into()),
        });
        self.choosing_path = true;
        self._prompt = Some(cx.spawn_in(window, async move |view, cx| {
            let result = prompt.await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.choosing_path = false;
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(folder) = paths.first() {
                            view.set_path(folder.join(name), window, cx);
                            view.selected_folder = Some(folder.clone());
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => window.push_notification("Could not open the folder picker", cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn set_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_folder = None;
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            self.filename
                .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
        }
        match path.to_str() {
            Some(path) => self
                .destination
                .update(cx, |input, cx| input.set_value(path.to_owned(), window, cx)),
            None => window
                .push_notification("This filename cannot be represented in the path input", cx),
        }
    }
}

impl Render for AddDownload {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().gap_3().text_color(cx.theme().foreground)
            .child(Form::new()
                .child(Field::new().label("Address").child(Input::new(&self.source).disabled(self.choosing_path)))
                .child(Field::new().label("File name").description("Detected from the URL. Unsafe filename characters are replaced when saved.").child(Input::new(&self.filename).disabled(self.choosing_path)))
                .child(Field::new().label("Save as").child(Input::new(&self.destination).disabled(self.choosing_path))))
            .child(div().flex().gap_2()
                .child(Button::new("choose-folder").label("Choose folder…").disabled(self.choosing_path).on_click(cx.listener(|view, _, window, cx| view.choose_folder(window, cx))))
                .child(Button::new("choose-destination").label("Save as…").disabled(self.choosing_path).on_click(cx.listener(|view, _, window, cx| view.choose_path(window, cx)))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("No overwrite. HTTP / HTTPS. Nothing is downloaded until you confirm."))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("close-add").label("Cancel").disabled(self.choosing_path).on_click(cx.listener(|_, _, _, cx| cx.emit(DismissAdd))))
                .child(Button::new("enqueue-download").label("Download later").disabled(!self.can_submit(cx)).on_click(cx.listener(|view, _, _, cx| view.submit(false, cx))))
                .child(Button::new("download-now").primary().label("Start download").disabled(!self.can_submit(cx)).on_click(cx.listener(|view, _, _, cx| view.submit(true, cx)))))
    }
}
