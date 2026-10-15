use super::*;
use crate::{
    native_windows::{NativeSchedulerWindow, child_options},
    scheduler::{Schedule, ScheduleAction},
    scheduler_window::{QueueMember, QueueSummary, SchedulerRequest, SchedulerSnapshot},
};

impl DownloadManager {
    pub(super) fn scheduler_snapshot(&self) -> SchedulerSnapshot {
        let mut members: Vec<_> = self
            .queue
            .items
            .iter()
            .filter(|item| item.queue_id == self.scheduler.selected)
            .collect();
        members.sort_by_key(|item| (item.queue_order, item.id));
        SchedulerSnapshot {
            queues: self
                .scheduler
                .queues
                .iter()
                .map(|queue| QueueSummary {
                    id: queue.id,
                    name: queue.name.clone(),
                    running: if queue.id == DEFAULT_QUEUE {
                        self.queue_running
                    } else {
                        queue.running
                    },
                    schedule: queue
                        .schedule
                        .as_ref()
                        .filter(|schedule| schedule.enabled)
                        .map(|schedule| schedule.spec.clone()),
                })
                .collect(),
            selected: self.scheduler.selected,
            members: members
                .into_iter()
                .map(|item| QueueMember {
                    id: item.id,
                    name: item.name(),
                    status: item.status().into(),
                    movable: item.is_queued() && self.active != Some(item.id),
                    description: item.description.clone(),
                })
                .collect(),
            selected_downloads: self.queue.selected.len(),
            error: self.scheduler_error.clone(),
        }
    }

    pub(crate) fn open_scheduler(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(child) = &self.scheduler_window
            && cx.windows().contains(&child.handle)
        {
            let handle = child.handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }
        if self.options_open || self.about_open || self.removal_open || self.removal_busy {
            return;
        }
        self.scheduler_window = None;
        self.scheduler_error = None;
        let owner = cx.entity();
        let snapshot = self.scheduler_snapshot();
        match open_window(
            child_options("Scheduling & queues", 740., 720., cx),
            cx,
            move |window, cx| cx.new(|cx| NativeSchedulerWindow::new(owner, snapshot, window, cx)),
        ) {
            Ok((handle, root)) => {
                let content = root.read(cx).content.downgrade();
                self.scheduler_window = Some(ChildWindow { handle, content });
                #[cfg(any(target_os = "macos", test))]
                cx.emit(MenuStateChanged);
            }
            Err(_) => {
                self.notice = Some("Could not open Scheduling & queues.".into());
                cx.notify();
            }
        }
    }

    pub(super) fn refresh_scheduler(&self, cx: &mut Context<Self>) {
        let Some(child) = &self.scheduler_window else {
            return;
        };
        let handle = child.handle;
        let content = child.content.clone();
        let snapshot = self.scheduler_snapshot();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ =
                    content.update(cx, |content, cx| content.set_snapshot(snapshot, window, cx));
            });
        });
    }

    pub(crate) fn scheduler_request(
        &mut self,
        request: &SchedulerRequest,
        draft: &SchedulerSnapshot,
        cx: &mut Context<Self>,
    ) {
        if self.removal_busy || self.removal_open || self.options_open || self.about_open {
            self.scheduler_error = Some("Finish the other dialog or file removal first.".into());
            self.refresh_scheduler(cx);
            return;
        }
        let result = self.apply_scheduler_request(request, draft, cx);
        self.scheduler_error = result.err().map(str::to_owned);
        self.sync_table(cx);
        cx.notify();
    }

    fn apply_scheduler_request(
        &mut self,
        request: &SchedulerRequest,
        draft: &SchedulerSnapshot,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        use SchedulerRequest::*;
        if draft.selected != self.scheduler.selected {
            return Err("Queue changed. Review the current queue and try again.");
        }
        let selected = self.scheduler.selected;
        match request {
            SelectQueue(id) => {
                self.scheduler.queue(*id).ok_or("Queue not found")?;
                self.scheduler.selected = *id;
            }
            CreateQueue(name) => {
                let id = self.scheduler.create(name)?;
                self.scheduler.selected = id;
            }
            RenameQueue(id, name) if *id == selected => self.scheduler.rename(*id, name)?,
            DeleteQueue(id) if *id == selected => {
                if *id == DEFAULT_QUEUE {
                    return Err("The main queue cannot be removed");
                }
                let ids: Vec<_> = self
                    .queue
                    .items
                    .iter()
                    .filter(|item| item.queue_id == *id)
                    .map(|item| item.id)
                    .collect();
                if ids.iter().any(|id| self.active == Some(*id)) {
                    return Err("Stop active work before removing its queue");
                }
                self.queue.assign_queue(&ids, DEFAULT_QUEUE)?;
                self.scheduler.remove(*id)?;
                self.cancel_all_shutdowns(cx);
            }
            AssignSelected(id) if *id == selected => {
                self.scheduler.queue(*id).ok_or("Queue not found")?;
                let ids: Vec<_> = self.queue.selected.iter().copied().collect();
                if ids.is_empty() {
                    return Err("Select downloads in the main table first");
                }
                self.queue.assign_queue(&ids, *id)?;
                self.cancel_all_shutdowns(cx);
                self.advance(cx);
            }
            Move(id, earlier) => {
                if !draft
                    .members
                    .iter()
                    .any(|member| member.id == *id && member.movable)
                    || !self
                        .queue
                        .item(*id)
                        .is_some_and(|item| item.queue_id == selected && item.is_queued())
                    || self.active == Some(*id)
                {
                    return Err("Only waiting downloads in this queue can be moved");
                }
                if !self.queue.move_queued(*id, *earlier) {
                    return Err("This download is already at that end of the queue");
                }
            }
            SetDescription(id, description) => {
                if !draft.members.iter().any(|member| member.id == *id)
                    || !self
                        .queue
                        .item(*id)
                        .is_some_and(|item| item.queue_id == selected)
                {
                    return Err("Download is no longer in this queue");
                }
                self.queue.set_description(*id, description)?;
            }
            StartQueue(id) if *id == selected => {
                self.scheduler.queue(*id).ok_or("Queue not found")?;
                if self.queue.next_in_queue(*id).is_none() {
                    return Err("This queue has no waiting downloads");
                }
                self.cancel_all_shutdowns(cx);
                self.scheduler.queue_mut(*id).unwrap().running = true;
                if *id == DEFAULT_QUEUE {
                    self.queue_running = true;
                }
                self.advance(cx);
            }
            StopQueue(id) if *id == selected => {
                self.scheduler
                    .queue_mut(*id)
                    .ok_or("Queue not found")?
                    .running = false;
                if *id == DEFAULT_QUEUE {
                    self.queue_running = false;
                }
                self.notice = Some("Queue stopped. Its active download will finish.".into());
                self.cancel_all_shutdowns(cx);
            }
            SetSchedule(id, spec) if *id == selected => {
                let now = chrono::Local::now();
                let schedule = Schedule::new(spec.clone(), now)?;
                let summary = schedule.summary();
                self.scheduler.set_schedule(*id, Some(schedule), now)?;
                self.cancel_all_shutdowns(cx);
                self.notice = Some(format!("Schedule set: {summary}. Ulos must stay open."));
            }
            ClearSchedule(id) if *id == selected => {
                self.scheduler
                    .set_schedule(*id, None, chrono::Local::now())?
            }
            Close => (),
            _ => return Err("Queue changed. Review it before applying this action."),
        }
        Ok(())
    }

    pub(super) fn scheduler_tick(
        &mut self,
        now: chrono::DateTime<chrono::Local>,
        cx: &mut Context<Self>,
    ) {
        // Even when file removal blocks dispatch, consume elapsed occurrences so an
        // expired window cannot replay later. Stop events always withdraw advancement.
        let enabled_before: Vec<_> = self
            .scheduler
            .queues
            .iter()
            .map(|queue| {
                queue
                    .schedule
                    .as_ref()
                    .is_some_and(|schedule| schedule.enabled)
            })
            .collect();
        let actions = self.scheduler.tick(now);
        if actions.is_empty() {
            if self
                .scheduler
                .queues
                .iter()
                .zip(enabled_before)
                .any(|(queue, before)| {
                    before
                        != queue
                            .schedule
                            .as_ref()
                            .is_some_and(|schedule| schedule.enabled)
                })
            {
                self.refresh_scheduler(cx);
                cx.notify();
            }
            return;
        }
        for (id, action) in actions {
            if id == DEFAULT_QUEUE {
                self.queue_running = action == ScheduleAction::Start;
            }
            self.cancel_all_shutdowns(cx);
        }
        self.advance(cx);
        self.sync_table(cx);
        cx.notify();
    }
}
