//! Snapshot-only scheduler content. The owning native root validates and executes every intent.
use chrono::{NaiveDate, NaiveTime};
use gpui_kit::{
    component::{
        Disableable,
        button::Button,
        checkbox::Checkbox,
        form::{Field, Form},
        h_flex,
        input::{Input, InputEvent, InputState},
        label::Label,
        menu::{DropdownMenu, PopupMenuItem},
        radio::{Radio, RadioGroup},
        toolbar::Toolbar,
        v_flex,
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::scheduler::{QueueId, ScheduleSpec};

#[derive(Clone, PartialEq, Eq)]
pub struct SchedulerSnapshot {
    pub queues: Vec<QueueSummary>,
    pub selected: QueueId,
    pub members: Vec<QueueMember>,
    pub selected_downloads: usize,
    pub error: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct QueueSummary {
    pub id: QueueId,
    pub name: String,
    pub running: bool,
    pub schedule: Option<ScheduleSpec>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct QueueMember {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub movable: bool,
    pub description: String,
}

#[derive(Clone, PartialEq, Eq)]
pub enum SchedulerRequest {
    SelectQueue(QueueId),
    CreateQueue(String),
    RenameQueue(QueueId, String),
    DeleteQueue(QueueId),
    StartQueue(QueueId),
    StopQueue(QueueId),
    AssignSelected(QueueId),
    Move(u64, bool),
    SetDescription(u64, String),
    SetSchedule(QueueId, ScheduleSpec),
    ClearSchedule(QueueId),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScheduleMode {
    Once,
    Weekly,
}

pub struct SchedulerWindow {
    pub snapshot: SchedulerSnapshot,
    new_name: Entity<InputState>,
    rename: Entity<InputState>,
    description: Entity<InputState>,
    date: Entity<InputState>,
    start: Entity<InputState>,
    stop: Entity<InputState>,
    member: Option<u64>,
    mode: ScheduleMode,
    days: [bool; 7],
    input_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SchedulerRequest> for SchedulerWindow {}

// Check exact syntax, rather than accepting chrono's more permissive numeric widths.
fn parse_date(text: &str) -> Result<NaiveDate, String> {
    let text = text.trim();
    if text.len() != 10
        || text.as_bytes()[4] != b'-'
        || text.as_bytes()[7] != b'-'
        || !text
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err("Date must be YYYY-MM-DD.".into());
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| "Enter a valid calendar date (YYYY-MM-DD).".into())
}

fn parse_time(text: &str) -> Result<NaiveTime, String> {
    let text = text.trim();
    if text.len() != 5
        || text.as_bytes()[2] != b':'
        || !text
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 2 || b.is_ascii_digit())
    {
        return Err("Times must be HH:MM in 24-hour local time.".into());
    }
    NaiveTime::parse_from_str(text, "%H:%M")
        .map_err(|_| "Enter a valid time from 00:00 to 23:59.".into())
}

impl SchedulerWindow {
    pub fn new(snapshot: SchedulerSnapshot, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let new_name = cx.new(|cx| InputState::new(window, cx).placeholder("New queue name"));
        let rename = cx.new(|cx| InputState::new(window, cx));
        let description =
            cx.new(|cx| InputState::new(window, cx).placeholder("Download description"));
        let date = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        let start = cx.new(|cx| InputState::new(window, cx).placeholder("HH:MM"));
        let stop = cx.new(|cx| InputState::new(window, cx).placeholder("Optional HH:MM"));
        let subscriptions = [&new_name, &rename, &description, &date, &start, &stop]
            .into_iter()
            .map(|input| {
                cx.subscribe_in(input, window, |_, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        let mut view = Self {
            snapshot,
            new_name,
            rename,
            description,
            date,
            start,
            stop,
            member: None,
            mode: ScheduleMode::Once,
            days: [false; 7],
            input_error: None,
            _subscriptions: subscriptions,
        };
        view.load_queue_drafts(window, cx);
        view
    }

    fn queue(&self) -> Option<&QueueSummary> {
        self.snapshot
            .queues
            .iter()
            .find(|q| q.id == self.snapshot.selected)
    }

    fn set_input(
        input: &Entity<InputState>,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }

    fn load_queue_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let queue = self.queue().cloned();
        Self::set_input(
            &self.rename,
            queue.as_ref().map(|q| q.name.clone()).unwrap_or_default(),
            window,
            cx,
        );
        self.mode = ScheduleMode::Once;
        self.days = [false; 7];
        let mut date = String::new();
        let mut start = String::new();
        let mut stop = String::new();
        if let Some(schedule) = queue.and_then(|q| q.schedule) {
            match schedule {
                ScheduleSpec::Once {
                    date: on,
                    start: at,
                    stop: until,
                } => {
                    date = on.format("%Y-%m-%d").to_string();
                    start = at.format("%H:%M").to_string();
                    stop = until
                        .map(|t| t.format("%H:%M").to_string())
                        .unwrap_or_default();
                }
                ScheduleSpec::Weekly {
                    days,
                    start: at,
                    stop: until,
                } => {
                    self.mode = ScheduleMode::Weekly;
                    self.days = days;
                    start = at.format("%H:%M").to_string();
                    stop = until
                        .map(|t| t.format("%H:%M").to_string())
                        .unwrap_or_default();
                }
            }
        }
        Self::set_input(&self.date, date, window, cx);
        Self::set_input(&self.start, start, window, cx);
        Self::set_input(&self.stop, stop, window, cx);
        self.member = None;
        self.select_member(self.snapshot.members.first().map(|m| m.id), window, cx);
        self.input_error = None;
    }

    pub fn set_snapshot(
        &mut self,
        snapshot: SchedulerSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot {
            return;
        }
        let changed_queue = self.snapshot.selected != snapshot.selected;
        self.snapshot = snapshot;
        if changed_queue {
            self.load_queue_drafts(window, cx);
        } else if !self
            .snapshot
            .members
            .iter()
            .any(|m| Some(m.id) == self.member)
        {
            self.select_member(self.snapshot.members.first().map(|m| m.id), window, cx);
        }
        cx.notify();
    }

    fn select_member(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        self.member = id;
        let description = self
            .snapshot
            .members
            .iter()
            .find(|m| Some(m.id) == id)
            .map(|m| m.description.clone())
            .unwrap_or_default();
        Self::set_input(&self.description, description, window, cx);
        cx.notify();
    }

    fn schedule_request(&self, cx: &App) -> Result<SchedulerRequest, String> {
        if self.queue().is_none() {
            return Err("Select an available queue.".into());
        }
        let start = parse_time(&self.start.read(cx).value())?;
        let stop_text = self.stop.read(cx).value();
        let stop = if stop_text.trim().is_empty() {
            None
        } else {
            Some(parse_time(&stop_text)?)
        };
        let schedule = match self.mode {
            ScheduleMode::Once => {
                let date = parse_date(&self.date.read(cx).value())?;
                ScheduleSpec::Once { date, start, stop }
            }
            ScheduleMode::Weekly => {
                if !self.days.iter().any(|day| *day) {
                    return Err("Choose at least one weekday.".into());
                }
                ScheduleSpec::Weekly {
                    days: self.days,
                    start,
                    stop,
                }
            }
        };
        Ok(SchedulerRequest::SetSchedule(
            self.snapshot.selected,
            schedule,
        ))
    }

    fn apply_schedule(&mut self, cx: &mut Context<Self>) {
        match self.schedule_request(cx) {
            Ok(request) => {
                self.input_error = None;
                cx.emit(request);
            }
            Err(error) => self.input_error = Some(error),
        }
        cx.notify();
    }

    fn name_request(&mut self, create: bool, cx: &mut Context<Self>) {
        let input = if create { &self.new_name } else { &self.rename };
        let name = input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.input_error = Some("Enter a queue name.".into());
        } else {
            self.input_error = None;
            cx.emit(if create {
                SchedulerRequest::CreateQueue(name)
            } else {
                SchedulerRequest::RenameQueue(self.snapshot.selected, name)
            });
        }
        cx.notify();
    }

    fn queue_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let queues = self.snapshot.queues.clone();
        let selected = self.snapshot.selected;
        let owner = cx.entity().downgrade();
        Button::new("scheduler-queue")
            .debug_selector(|| "scheduler-queue".into())
            .label(
                self.queue()
                    .map(|q| q.name.clone())
                    .unwrap_or_else(|| "Select queue".into()),
            )
            .accessibility_label("Selected queue")
            .dropdown_menu(move |menu, _, _| {
                queues.iter().fold(menu, |menu, queue| {
                    let owner = owner.clone();
                    let id = queue.id;
                    menu.item(
                        PopupMenuItem::new(queue.name.clone())
                            .checked(id == selected)
                            .on_click(move |_, _, cx| {
                                let _ = owner
                                    .update(cx, |_, cx| cx.emit(SchedulerRequest::SelectQueue(id)));
                            }),
                    )
                })
            })
    }

    fn member_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let members = self.snapshot.members.clone();
        let selected = self.member;
        let owner = cx.entity().downgrade();
        let label = members
            .iter()
            .position(|m| Some(m.id) == selected)
            .map(|i| format!("{}. {} — {}", i + 1, members[i].name, members[i].status))
            .unwrap_or_else(|| "No downloads in this queue".into());
        // Menu entries are constructed only when opened, not 500 eager row Buttons.
        Button::new("scheduler-member")
            .debug_selector(|| "scheduler-member".into())
            .label(label)
            .accessibility_label("Queue downloads in execution order")
            .disabled(members.is_empty())
            .dropdown_menu(move |menu, _, _| {
                members.iter().enumerate().fold(menu, |menu, (i, member)| {
                    let owner = owner.clone();
                    let id = member.id;
                    menu.item(
                        PopupMenuItem::new(format!(
                            "{}. {} — {}",
                            i + 1,
                            member.name,
                            member.status
                        ))
                        .checked(Some(id) == selected)
                        .on_click(move |_, window, cx| {
                            let _ = owner
                                .update(cx, |view, cx| view.select_member(Some(id), window, cx));
                        }),
                    )
                })
            })
    }
}

impl Render for SchedulerWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.snapshot.selected;
        let available = self.queue().is_some();
        let running = self.queue().is_some_and(|q| q.running);
        let scheduled = self.queue().is_some_and(|q| q.schedule.is_some());
        let position = self
            .snapshot
            .members
            .iter()
            .position(|m| Some(m.id) == self.member);
        let movable = position.is_some_and(|i| self.snapshot.members[i].movable);
        let earlier = movable
            && position.is_some_and(|i| self.snapshot.members[..i].iter().any(|m| m.movable));
        let later = movable
            && position.is_some_and(|i| self.snapshot.members[i + 1..].iter().any(|m| m.movable));
        let member_id = self.member;
        v_flex().size_full().p_4().gap_3()
            .child(v_flex().id("scheduler-scroll").flex_1().min_h_0().overflow_y_scroll().gap_3()
                .child(Label::new("Queues and schedules work only while Ulos is open. Times use this computer’s local time."))
                .child(Label::new("One transfer runs at a time across all queues. Stop queue lets the current transfer finish."))
                .child(Form::new()
                    .child(Field::new().label("Queue").child(Toolbar::new("scheduler-selector-toolbar").content(self.queue_selector(cx))))
                    .child(Field::new().label("Queue state").child(Label::new(if running { "Running" } else { "Stopped" })))
                    .child(Field::new().label("Create queue").child(Input::new(&self.new_name).id("scheduler-new-name")))
                    .child(Field::new().child(Toolbar::new("scheduler-create-toolbar").content(Button::new("scheduler-create").debug_selector(|| "scheduler-create".into()).label("Create queue").on_click(cx.listener(|view, _, _, cx| view.name_request(true, cx))))))
                    .child(Field::new().label("Queue name").child(Input::new(&self.rename).id("scheduler-rename-name").disabled(!available))))
                .child(Toolbar::new("scheduler-queue-actions").content(h_flex().flex_wrap().gap_2()
                    .child(Button::new("scheduler-rename").debug_selector(|| "scheduler-rename".into()).label("Rename").disabled(!available).on_click(cx.listener(|view, _, _, cx| view.name_request(false, cx))))
                    .child(Button::new("scheduler-delete").debug_selector(|| "scheduler-delete".into()).label("Delete queue").disabled(!available || id == crate::scheduler::DEFAULT_QUEUE).on_click(cx.listener(move |_, _, _, cx| cx.emit(SchedulerRequest::DeleteQueue(id)))))
                    .child(Button::new("scheduler-start").debug_selector(|| "scheduler-start".into()).label("Start queue").disabled(!available || running).on_click(cx.listener(move |_, _, _, cx| cx.emit(SchedulerRequest::StartQueue(id)))))
                    .child(Button::new("scheduler-stop").debug_selector(|| "scheduler-stop".into()).label("Stop queue").disabled(!running).on_click(cx.listener(move |_, _, _, cx| cx.emit(SchedulerRequest::StopQueue(id)))))))
                .child(Toolbar::new("scheduler-assignment-toolbar").content(Button::new("scheduler-assign").debug_selector(|| "scheduler-assign".into()).label(format!("Assign selected downloads ({})", self.snapshot.selected_downloads)).disabled(!available || self.snapshot.selected_downloads == 0).on_click(cx.listener(move |_, _, _, cx| cx.emit(SchedulerRequest::AssignSelected(id))))))
                .child(Label::new("Main queue cannot be deleted. Active downloads cannot be moved."))
                .child(Form::new()
                    .child(Field::new().label(format!("Downloads in queue ({})", self.snapshot.members.len())).child(Toolbar::new("scheduler-members-toolbar").content(self.member_selector(cx))))
                    .child(Field::new().child(Toolbar::new("scheduler-move-toolbar").content(h_flex().gap_2()
                        .child(Button::new("scheduler-earlier").debug_selector(|| "scheduler-earlier".into()).label("Move earlier").disabled(!earlier).on_click(cx.listener(move |_, _, _, cx| { if let Some(id) = member_id { cx.emit(SchedulerRequest::Move(id, true)); } })))
                        .child(Button::new("scheduler-later").debug_selector(|| "scheduler-later".into()).label("Move later").disabled(!later).on_click(cx.listener(move |_, _, _, cx| { if let Some(id) = member_id { cx.emit(SchedulerRequest::Move(id, false)); } }))))))
                    .child(Field::new().label("Description").child(Input::new(&self.description).id("scheduler-description").disabled(member_id.is_none())))
                    .child(Field::new().child(Toolbar::new("scheduler-description-toolbar").content(Button::new("scheduler-description-apply").debug_selector(|| "scheduler-description-apply".into()).label("Apply description").disabled(member_id.is_none()).on_click(cx.listener(move |view, _, _, cx| { if let Some(id) = member_id { cx.emit(SchedulerRequest::SetDescription(id, view.description.read(cx).value().to_string())); } }))))))
                .child(Form::new()
                    .child(Field::new().label("Schedule").child(RadioGroup::horizontal("scheduler-mode").selected_index(Some(if self.mode == ScheduleMode::Once { 0 } else { 1 }))
                        .children([Radio::new("scheduler-once").debug_selector(|| "scheduler-once".into()).label("Once"), Radio::new("scheduler-weekly").debug_selector(|| "scheduler-weekly".into()).label("Weekly")])
                        .on_change(cx.listener(|view, index: &usize, _, cx| { view.mode = if *index == 1 { ScheduleMode::Weekly } else { ScheduleMode::Once }; cx.notify(); }))))
                    .when(self.mode == ScheduleMode::Once, |form| form.child(Field::new().label("Date (YYYY-MM-DD)").child(Input::new(&self.date).id("scheduler-date"))))
                    .when(self.mode == ScheduleMode::Weekly, |form| form.child(Field::new().label("Weekdays").child(h_flex().flex_wrap().gap_2().children(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].into_iter().enumerate().map(|(i, day)| {
                        Checkbox::new(("scheduler-day", i)).debug_selector(move || format!("scheduler-day-{i}")).label(day).checked(self.days[i]).on_click(cx.listener(move |view, checked: &bool, _, cx| { view.days[i] = *checked; cx.notify(); }))
                    })))))
                    .child(Field::new().label("Start (HH:MM)").child(Input::new(&self.start).id("scheduler-start-time")))
                    .child(Field::new().label("Stop (optional HH:MM)").child(Input::new(&self.stop).id("scheduler-stop-time"))))
                .child(Toolbar::new("scheduler-schedule-toolbar").content(h_flex().gap_2()
                    .child(Button::new("scheduler-apply").debug_selector(|| "scheduler-apply".into()).label("Apply schedule").disabled(!available).on_click(cx.listener(|view, _, _, cx| view.apply_schedule(cx))))
                    .child(Button::new("scheduler-clear").debug_selector(|| "scheduler-clear".into()).label("Clear schedule").disabled(!scheduled).on_click(cx.listener(move |_, _, _, cx| cx.emit(SchedulerRequest::ClearSchedule(id)))))))
                .child(Label::new("A stop earlier than start is on the following day. Scheduled stop does not cancel current work."))
                .child(Label::new(if scheduled { "A schedule is applied. Form changes take effect only after Apply schedule." } else { "No schedule is applied. Form changes take effect only after Apply schedule." }))
                .when_some(self.input_error.clone(), |body, error| body.child(Label::new(error)))
                .when_some(self.snapshot.error.clone(), |body, error| body.child(Label::new(error))))
            .child(Toolbar::new("scheduler-footer").content(Button::new("scheduler-close").debug_selector(|| "scheduler-close".into()).label("Close").on_click(cx.listener(|_, _, _, cx| cx.emit(SchedulerRequest::Close)))))
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_date, parse_time};

    #[test]
    fn dates_and_times_require_exact_local_syntax() {
        assert!(parse_date("2028-02-29").is_ok());
        for text in ["2027-02-29", "2027-2-09", "2027/02/09", "", "💥"] {
            assert!(parse_date(text).is_err());
        }
        for text in ["00:00", "23:59", " 09:30 "] {
            assert!(parse_time(text).is_ok());
        }
        for text in ["24:00", "12:60", "9:30", "09:30:00", "💥", ""] {
            assert!(parse_time(text).is_err());
        }
    }
}

#[cfg(test)]
mod window_tests {
    use super::{
        QueueMember, QueueSummary, ScheduleMode, ScheduleSpec, SchedulerRequest, SchedulerSnapshot,
        SchedulerWindow,
    };
    use gpui_kit::{AppContext as _, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    fn snapshot() -> SchedulerSnapshot {
        SchedulerSnapshot {
            queues: vec![QueueSummary {
                id: 0,
                name: "Main queue".into(),
                running: false,
                schedule: None,
            }],
            selected: 0,
            members: vec![QueueMember {
                id: 7,
                name: "Download".into(),
                status: "Queued".into(),
                movable: true,
                description: String::new(),
            }],
            selected_downloads: 0,
            error: None,
        }
    }

    #[gpui::test]
    fn drafts_survive_refresh_and_only_valid_forms_emit(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let mut content = None;
        let (_root, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| SchedulerWindow::new(snapshot(), window, cx));
            content = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let view = content.unwrap();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let received = requests.clone();
        let _subscription = cx.update(|_, cx| {
            cx.subscribe(&view, move |_, event: &SchedulerRequest, _| {
                received.borrow_mut().push(event.clone());
            })
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                SchedulerWindow::set_input(&view.date, "2030-01-02".into(), window, cx);
                SchedulerWindow::set_input(&view.start, "bad time".into(), window, cx);
                SchedulerWindow::set_input(&view.rename, "Unsaved name".into(), window, cx);
                SchedulerWindow::set_input(
                    &view.description,
                    "Unsaved description".into(),
                    window,
                    cx,
                );
                view.apply_schedule(cx);
            })
        });
        cx.run_until_parked();
        assert!(requests.borrow().is_empty());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(view.input_error.is_some());
                let mut changed = snapshot();
                changed.queues[0].running = true;
                changed.members[0].status = "Downloading".into();
                view.set_snapshot(changed, window, cx);
                assert_eq!(view.rename.read(cx).value().as_ref(), "Unsaved name");
                assert_eq!(
                    view.description.read(cx).value().as_ref(),
                    "Unsaved description"
                );
                assert_eq!(view.start.read(cx).value().as_ref(), "bad time");
                SchedulerWindow::set_input(&view.start, "09:30".into(), window, cx);
                view.apply_schedule(cx);
            })
        });
        cx.run_until_parked();
        assert!(matches!(
            requests.borrow().as_slice(),
            [SchedulerRequest::SetSchedule(0, ScheduleSpec::Once { .. })]
        ));
        requests.borrow_mut().clear();
        cx.update(|_window, cx| {
            view.update(cx, |view, cx| {
                view.mode = ScheduleMode::Weekly;
                view.days = [false; 7];
                view.apply_schedule(cx);
                assert!(view.input_error.is_some());
            })
        });
        cx.run_until_parked();
        assert!(requests.borrow().is_empty());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.days[0] = true;
                view.apply_schedule(cx);
                let mut changed = snapshot();
                changed.queues.push(QueueSummary {
                    id: 1,
                    name: "Other".into(),
                    running: false,
                    schedule: None,
                });
                changed.selected = 1;
                changed.members.clear();
                view.set_snapshot(changed, window, cx);
                assert_eq!(view.rename.read(cx).value().as_ref(), "Other");
                assert!(view.start.read(cx).value().is_empty());
                assert!(view.description.read(cx).value().is_empty());
                assert!(view.mode == ScheduleMode::Once);
                assert_eq!(view.days, [false; 7]);
            })
        });
        cx.run_until_parked();
        assert!(matches!(
            requests.borrow().as_slice(),
            [SchedulerRequest::SetSchedule(
                0,
                ScheduleSpec::Weekly {
                    days: [true, false, false, false, false, false, false],
                    ..
                }
            )]
        ));
    }
}
