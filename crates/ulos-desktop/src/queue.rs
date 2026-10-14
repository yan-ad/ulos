//! Bounded, session-only queue presentation model. No networking or file operations.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use ulos_core::{DownloadProgress, tasks::TransferState};

pub type DownloadId = u64;
pub const MAX_ITEMS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Image,
    Music,
    Video,
    Apps,
    Document,
    Compressed,
    Other,
}

impl Category {
    pub const ALL: [Self; 7] = [
        Self::Image,
        Self::Music,
        Self::Video,
        Self::Apps,
        Self::Document,
        Self::Compressed,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Music => "Music",
            Self::Video => "Video",
            Self::Apps => "Apps",
            Self::Document => "Document",
            Self::Compressed => "Compressed",
            Self::Other => "Other",
        }
    }

    pub fn from_path(path: &Path) -> Self {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "ico"
            | "heic" | "avif" => Self::Image,
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "opus" | "aiff" | "wma" => Self::Music,
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "wmv" => Self::Video,
            "exe" | "msi" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "apk" | "app" | "msix" => {
                Self::Apps
            }
            "pdf" | "txt" | "md" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "csv"
            | "rtf" | "odt" | "ods" | "epub" => Self::Document,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" | "tbz2" | "txz" => {
                Self::Compressed
            }
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Category(Category),
    Finished,
    Unfinished,
    Queued,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    #[default]
    Added,
}

// Deliberately no Debug: source URLs may contain private signed query strings.
pub struct QueueItem {
    pub id: DownloadId,
    pub destination: PathBuf,
    pub category: Category,
    pub added_at: SystemTime,
    pub state: Option<TransferState>,
    pub cancelling: bool,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    source: String,
    last_sample: Option<(Instant, u64)>,
    measured_rate: Option<f64>,
}

impl QueueItem {
    pub fn name(&self) -> String {
        self.destination
            .file_name()
            .unwrap_or(self.destination.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn status(&self) -> &str {
        if self.cancelling && self.is_active() {
            return "Cancelling";
        }
        match self.state.as_ref() {
            None => "Added",
            Some(TransferState::Preparing) => "Connecting",
            Some(TransferState::Downloading(_)) => "Downloading",
            Some(TransferState::Completed(_)) => "Finished",
            Some(TransferState::Failed(_)) => "Failed",
            Some(TransferState::Cancelled) => "Cancelled",
        }
    }

    pub fn is_active(&self) -> bool {
        self.state.as_ref().is_some_and(TransferState::is_active)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, Some(TransferState::Completed(_)))
    }

    pub fn is_queued(&self) -> bool {
        self.state.is_none()
    }

    pub fn progress(&self) -> Option<DownloadProgress> {
        if matches!(
            self.state,
            Some(TransferState::Downloading(_) | TransferState::Completed(_))
        ) || self.downloaded_bytes > 0
        {
            Some(DownloadProgress {
                downloaded_bytes: self.downloaded_bytes,
                total_bytes: self.total_bytes,
            })
        } else {
            None
        }
    }

    pub fn speed_bytes_per_second(&self) -> Option<f64> {
        if matches!(self.state, Some(TransferState::Downloading(_)))
            && self
                .last_sample
                .is_some_and(|(time, _)| time.elapsed() < Duration::from_secs(2))
        {
            self.measured_rate
        } else {
            None
        }
    }

    pub fn eta(&self) -> Option<Duration> {
        let total = self.total_bytes?;
        let rate = self.speed_bytes_per_second()?;
        if rate <= 0.0 || !rate.is_finite() {
            return None;
        }
        Duration::try_from_secs_f64(total.saturating_sub(self.downloaded_bytes) as f64 / rate).ok()
    }

    /// Apply a worker observation; terminal states are immutable, including when
    /// an older coalesced observation arrives after completion or cancellation.
    pub fn apply_state(&mut self, state: TransferState, now: Instant) {
        if self.state.as_ref().is_some_and(TransferState::is_terminal) {
            return;
        }
        match &state {
            TransferState::Downloading(progress) => {
                if let Some((previous, bytes)) = self.last_sample {
                    if let Some(elapsed) = now
                        .checked_duration_since(previous)
                        .filter(|value| !value.is_zero())
                    {
                        self.measured_rate = Some(
                            progress.downloaded_bytes.saturating_sub(bytes) as f64
                                / elapsed.as_secs_f64(),
                        );
                        self.last_sample = Some((now, progress.downloaded_bytes));
                    }
                } else {
                    self.last_sample = Some((now, progress.downloaded_bytes));
                }
                self.downloaded_bytes = progress.downloaded_bytes;
                self.total_bytes = progress.total_bytes;
            }
            TransferState::Completed(receipt) => {
                self.downloaded_bytes = receipt.downloaded_bytes;
                self.total_bytes = Some(receipt.downloaded_bytes);
            }
            TransferState::Preparing => {
                self.last_sample = None;
                self.measured_rate = None;
            }
            TransferState::Failed(_) | TransferState::Cancelled => {}
        }
        if state.is_terminal() {
            self.cancelling = false;
            self.last_sample = None;
            self.measured_rate = None;
        }
        self.state = Some(state);
    }

    /// This records intent only; the owner must signal the actual worker.
    pub fn request_cancel(&mut self) -> bool {
        if !self.is_active() || self.cancelling {
            return false;
        }
        self.cancelling = true;
        true
    }
}

#[derive(Default)]
pub struct SessionQueue {
    pub items: Vec<QueueItem>,
    pub selected: BTreeSet<DownloadId>,
    pub filter: Filter,
    pub search: String,
    pub sort: SortColumn,
    pub descending: bool,
    next_id: DownloadId,
}

impl SessionQueue {
    pub fn add(&mut self, url: String, destination: PathBuf) -> Result<DownloadId, &'static str> {
        if url.trim().is_empty() || destination.as_os_str().is_empty() {
            return Err("A source and destination are required");
        }
        if self.items.len() >= MAX_ITEMS {
            return Err("The session queue is full");
        }
        // Lexical check only: core no-clobber publication protects filesystem aliases.
        if self
            .items
            .iter()
            .any(|item| item.destination == destination && (item.is_queued() || item.is_active()))
        {
            return Err("This destination is already queued or active");
        }
        let id = self
            .next_id
            .checked_add(1)
            .ok_or("Download identifiers exhausted")?;
        self.next_id = id;
        self.items.push(QueueItem {
            id,
            category: Category::from_path(&destination),
            destination,
            added_at: SystemTime::now(),
            state: None,
            cancelling: false,
            downloaded_bytes: 0,
            total_bytes: None,
            source: url,
            last_sample: None,
            measured_rate: None,
        });
        Ok(id)
    }

    pub fn item(&self, id: DownloadId) -> Option<&QueueItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn item_mut(&mut self, id: DownloadId) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    pub fn next_queued(&self) -> Option<DownloadId> {
        self.items
            .iter()
            .filter(|item| item.is_queued())
            .min_by_key(|item| item.id)
            .map(|item| item.id)
    }

    pub fn active_count(&self) -> usize {
        self.items.iter().filter(|item| item.is_active()).count()
    }

    pub fn visible(&self) -> Vec<&QueueItem> {
        let search = self.search.to_lowercase();
        let mut visible: Vec<_> = self
            .items
            .iter()
            .filter(|item| {
                let matches_filter = match self.filter {
                    Filter::All => true,
                    Filter::Category(category) => item.category == category,
                    Filter::Finished => item.is_finished(),
                    Filter::Unfinished => !item.is_finished(),
                    Filter::Queued => item.is_queued(),
                };
                matches_filter && item.name().to_lowercase().contains(&search)
            })
            .collect();
        visible.sort_by(|left, right| {
            // Unknown sizes remain last in both directions.
            let order = match self.sort {
                SortColumn::Name => left.name().to_lowercase().cmp(&right.name().to_lowercase()),
                SortColumn::Added => left.added_at.cmp(&right.added_at),
                SortColumn::Size => match (left.total_bytes, right.total_bytes) {
                    (Some(_), None) => return std::cmp::Ordering::Less,
                    (None, Some(_)) => return std::cmp::Ordering::Greater,
                    (a, b) => a.cmp(&b),
                },
            };
            let order = if self.descending {
                order.reverse()
            } else {
                order
            };
            order.then_with(|| left.id.cmp(&right.id))
        });
        visible
    }

    pub fn select(&mut self, id: DownloadId, selected: bool) {
        if selected && self.item(id).is_some() {
            self.selected.insert(id);
        } else {
            self.selected.remove(&id);
        }
    }

    pub fn select_visible(&mut self, selected: bool) {
        let ids: Vec<_> = self.visible().into_iter().map(|item| item.id).collect();
        for id in ids {
            self.select(id, selected);
        }
    }

    pub fn all_visible_selected(&self) -> bool {
        let visible = self.visible();
        !visible.is_empty() && visible.iter().all(|item| self.selected.contains(&item.id))
    }

    /// Remove metadata only, never files; active rows retain their ownership.
    pub fn remove_selected(&mut self) -> usize {
        let before = self.items.len();
        self.items
            .retain(|item| !self.selected.contains(&item.id) || item.is_active());
        self.selected
            .retain(|id| self.items.iter().any(|item| item.id == *id));
        before - self.items.len()
    }

    pub fn total_speed(&self) -> f64 {
        self.items
            .iter()
            .filter_map(QueueItem::speed_bytes_per_second)
            .sum()
    }

    pub fn sort_by(&mut self, column: SortColumn) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            self.descending = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulos_core::DownloadReceipt;

    fn add(queue: &mut SessionQueue, path: &str) -> DownloadId {
        let id = queue
            .add(
                "https://example.invalid/fixture?private=value".into(),
                path.into(),
            )
            .unwrap();
        queue.item_mut(id).unwrap().added_at = SystemTime::UNIX_EPOCH + Duration::from_secs(id);
        id
    }

    fn downloading(bytes: u64, total: Option<u64>) -> TransferState {
        TransferState::Downloading(DownloadProgress {
            downloaded_bytes: bytes,
            total_bytes: total,
        })
    }

    fn completed(bytes: u64) -> TransferState {
        TransferState::Completed(DownloadReceipt {
            destination: "fixture".into(),
            downloaded_bytes: bytes,
        })
    }

    fn visible(queue: &SessionQueue) -> Vec<DownloadId> {
        queue.visible().iter().map(|item| item.id).collect()
    }

    #[test]
    fn categories_cover_common_extensions_case_and_archives() {
        for (path, category) in [
            ("photo.JPEG", Category::Image),
            ("audio.FLAC", Category::Music),
            ("movie.mkv", Category::Video),
            ("setup.AppImage", Category::Apps),
            ("thing.app", Category::Apps),
            ("book.PDF", Category::Document),
            ("backup.TAR.GZ", Category::Compressed),
            ("unknown", Category::Other),
        ] {
            assert_eq!(Category::from_path(Path::new(path)), category);
        }
        assert_eq!(Category::ALL.len(), 7);
        assert!(
            Category::ALL
                .iter()
                .all(|category| !category.label().is_empty())
        );
    }

    #[test]
    fn filters_distinguish_success_from_other_terminal_states() {
        let mut queue = SessionQueue::default();
        let added = add(&mut queue, "added.txt");
        let success = add(&mut queue, "success.png");
        let failed = add(&mut queue, "failed.txt");
        let cancelled = add(&mut queue, "cancelled.txt");
        let now = Instant::now();
        queue
            .item_mut(success)
            .unwrap()
            .apply_state(completed(10), now);
        queue
            .item_mut(failed)
            .unwrap()
            .apply_state(TransferState::Failed("fixture".into()), now);
        queue
            .item_mut(cancelled)
            .unwrap()
            .apply_state(TransferState::Cancelled, now);
        queue.filter = Filter::Finished;
        assert_eq!(visible(&queue), vec![success]);
        queue.filter = Filter::Unfinished;
        assert_eq!(visible(&queue), vec![added, failed, cancelled]);
        queue.filter = Filter::Category(Category::Image);
        assert_eq!(visible(&queue), vec![success]);
    }

    #[test]
    fn search_uses_filename_not_private_source_or_parent() {
        let mut queue = SessionQueue::default();
        let id = add(&mut queue, "parent/Annual.PDF");
        queue.search = "ANNUAL".into();
        assert_eq!(visible(&queue), vec![id]);
        for search in ["parent", "private=value"] {
            queue.search = search.into();
            assert!(queue.visible().is_empty());
        }
    }

    #[test]
    fn sorting_keeps_unknown_sizes_last_and_id_ties_stable() {
        let mut queue = SessionQueue::default();
        let unknown = add(&mut queue, "z");
        let small = add(&mut queue, "A");
        let large = add(&mut queue, "a");
        queue.item_mut(small).unwrap().total_bytes = Some(1);
        queue.item_mut(large).unwrap().total_bytes = Some(2);
        queue.sort_by(SortColumn::Size);
        assert_eq!(visible(&queue), vec![small, large, unknown]);
        queue.sort_by(SortColumn::Size);
        assert_eq!(visible(&queue), vec![large, small, unknown]);
        queue.sort_by(SortColumn::Name);
        assert!(!queue.descending);
        assert_eq!(visible(&queue), vec![small, large, unknown]);
        for item in &mut queue.items {
            item.added_at = SystemTime::UNIX_EPOCH;
        }
        queue.sort_by(SortColumn::Added);
        assert_eq!(visible(&queue), vec![unknown, small, large]);
    }

    #[test]
    fn selection_survives_filter_sort_and_selects_only_visible_rows() {
        let mut queue = SessionQueue::default();
        let image = add(&mut queue, "a.png");
        let document = add(&mut queue, "b.pdf");
        queue.select(image, true);
        queue.filter = Filter::Category(Category::Document);
        assert!(!queue.all_visible_selected());
        queue.select_visible(true);
        queue.sort_by(SortColumn::Name);
        assert_eq!(queue.selected, BTreeSet::from([image, document]));
        queue.select_visible(false);
        assert_eq!(queue.selected, BTreeSet::from([image]));
        queue.search = "missing".into();
        assert!(!queue.all_visible_selected());
        queue.select(999, true);
        assert!(!queue.selected.contains(&999));
    }

    #[test]
    fn duplicates_are_rejected_only_while_queued_or_active_and_fifo_ignores_view() {
        let mut queue = SessionQueue::default();
        let first = add(&mut queue, "first");
        let second = add(&mut queue, "second");
        assert!(queue.add("source".into(), "first".into()).is_err());
        queue
            .item_mut(first)
            .unwrap()
            .apply_state(TransferState::Preparing, Instant::now());
        assert!(queue.add("source".into(), "first".into()).is_err());
        queue.filter = Filter::Finished;
        queue.search = "missing".into();
        queue.sort_by(SortColumn::Name);
        assert_eq!(queue.next_queued(), Some(second));
        queue
            .item_mut(first)
            .unwrap()
            .apply_state(TransferState::Failed("fixture".into()), Instant::now());
        assert!(queue.add("source".into(), "first".into()).is_ok());
    }

    #[test]
    fn empty_inputs_and_session_capacity_are_bounded() {
        let mut queue = SessionQueue::default();
        assert!(queue.add("  ".into(), "path".into()).is_err());
        assert!(queue.add("source".into(), PathBuf::new()).is_err());
        for index in 0..MAX_ITEMS {
            add(&mut queue, &format!("fixture-{index}"));
        }
        assert_eq!(queue.items.len(), MAX_ITEMS);
        assert!(queue.add("source".into(), "overflow".into()).is_err());
    }

    #[test]
    fn removal_is_metadata_only_and_retains_active_cancelling_rows() {
        let mut queue = SessionQueue::default();
        let active = add(&mut queue, "active");
        let queued = add(&mut queue, "queued");
        let finished = add(&mut queue, "finished");
        let now = Instant::now();
        queue
            .item_mut(active)
            .unwrap()
            .apply_state(TransferState::Preparing, now);
        assert!(queue.item_mut(active).unwrap().request_cancel());
        queue
            .item_mut(finished)
            .unwrap()
            .apply_state(completed(1), now);
        queue.select_visible(true);
        queue.selected.insert(999);
        assert_eq!(queue.remove_selected(), 2);
        assert!(queue.item(queued).is_none());
        assert_eq!(queue.selected, BTreeSet::from([active]));
        assert_eq!(queue.active_count(), 1);
    }

    #[test]
    fn status_and_cancellation_are_honest_and_idempotent() {
        let mut queue = SessionQueue::default();
        let id = add(&mut queue, "fixture");
        let item = queue.item_mut(id).unwrap();
        assert_eq!(item.status(), "Added");
        assert!(item.progress().is_none());
        assert!(!item.request_cancel());
        assert!(item.source().contains("private=value"));
        item.apply_state(TransferState::Preparing, Instant::now());
        assert_eq!(item.status(), "Connecting");
        assert!(item.request_cancel());
        assert!(!item.request_cancel());
        assert_eq!(item.status(), "Cancelling");
        item.apply_state(TransferState::Cancelled, Instant::now());
        assert_eq!(item.status(), "Cancelled");
        assert!(!item.cancelling);
        assert!(!item.request_cancel());
    }

    #[test]
    fn speed_and_eta_require_two_real_samples_and_clamp_remaining_bytes() {
        let mut queue = SessionQueue::default();
        let id = add(&mut queue, "fixture");
        let now = Instant::now();
        let item = queue.item_mut(id).unwrap();
        item.apply_state(TransferState::Preparing, now);
        item.apply_state(downloading(100, Some(1000)), now + Duration::from_secs(10));
        assert_eq!(item.speed_bytes_per_second(), None);
        item.apply_state(downloading(100, Some(1000)), now + Duration::from_secs(10));
        assert_eq!(item.eta(), None);
        item.apply_state(downloading(300, Some(1000)), now + Duration::from_secs(12));
        assert_eq!(item.speed_bytes_per_second(), Some(100.0));
        assert_eq!(item.eta(), Some(Duration::from_secs(7)));
        item.apply_state(downloading(400, None), now + Duration::from_secs(13));
        assert_eq!(item.eta(), None);
        item.apply_state(downloading(500, Some(450)), now + Duration::from_secs(14));
        assert_eq!(item.eta(), Some(Duration::ZERO));
        item.apply_state(downloading(500, Some(900)), now + Duration::from_secs(15));
        assert_eq!(item.speed_bytes_per_second(), Some(0.0));
        assert_eq!(item.eta(), None);
        item.apply_state(downloading(400, Some(900)), now + Duration::from_secs(16));
        assert_eq!(item.speed_bytes_per_second(), Some(0.0));
        assert_eq!(queue.total_speed(), 0.0);
    }

    #[test]
    fn stalled_rate_samples_expire_without_new_bytes() {
        let mut queue = SessionQueue::default();
        let id = add(&mut queue, "fixture");
        let now = Instant::now();
        let item = queue.item_mut(id).unwrap();
        item.apply_state(downloading(1, Some(1000)), now - Duration::from_secs(4));
        item.apply_state(downloading(101, Some(1000)), now - Duration::from_secs(3));
        assert_eq!(item.speed_bytes_per_second(), None);
        assert_eq!(item.eta(), None);
    }

    #[test]
    fn terminal_receipts_win_and_late_updates_are_ignored() {
        for terminal in [
            completed(42),
            TransferState::Failed("fixture".into()),
            TransferState::Cancelled,
        ] {
            let mut queue = SessionQueue::default();
            let id = add(&mut queue, "fixture");
            let now = Instant::now();
            let item = queue.item_mut(id).unwrap();
            item.apply_state(downloading(1, None), now);
            item.apply_state(downloading(11, None), now + Duration::from_secs(1));
            assert_eq!(queue.total_speed(), 10.0);
            let item = queue.item_mut(id).unwrap();
            assert!(item.request_cancel());
            item.apply_state(terminal.clone(), now + Duration::from_secs(2));
            item.apply_state(downloading(999, Some(1000)), now + Duration::from_secs(3));
            assert_eq!(item.state, Some(terminal.clone()));
            assert!(!item.cancelling);
            assert_eq!(item.speed_bytes_per_second(), None);
            if matches!(terminal, TransferState::Completed(_)) {
                assert_eq!(item.status(), "Finished");
                assert_eq!(
                    item.progress(),
                    Some(DownloadProgress {
                        downloaded_bytes: 42,
                        total_bytes: Some(42)
                    })
                );
            }
        }
    }
}
