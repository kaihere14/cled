use crate::Snapshot;

/// Decides whether a clipboard observation is a new change.
///
/// Kept separate from the polling loop so the logic is testable without a real clipboard.
#[derive(Debug, Default)]
pub(crate) struct ChangeDetector {
    last: Option<u64>,
}

impl ChangeDetector {
    /// Records the initial clipboard state without reporting it as a change.
    pub(crate) fn baseline(&mut self, snapshot: &Snapshot) {
        self.last = snapshot.fingerprint();
    }

    /// Returns `true` if `snapshot` differs from the previous observation.
    ///
    /// An empty clipboard is never a change, but it resets the state, so copying the same thing
    /// again after the clipboard was cleared still counts.
    pub(crate) fn observe(&mut self, snapshot: &Snapshot) -> bool {
        let current = snapshot.fingerprint();
        let changed = current.is_some() && current != self.last;
        self.last = current;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipboardContent, Image, SkipReason};

    fn text(s: &str) -> Snapshot {
        Snapshot::Content(ClipboardContent::text(s))
    }

    fn image(pixel: u8) -> Snapshot {
        Snapshot::Content(ClipboardContent::Image(
            Image::from_rgba(1, 1, vec![pixel; 4]).unwrap(),
        ))
    }

    const SENSITIVE: Snapshot = Snapshot::Skipped(SkipReason::Sensitive);

    #[test]
    fn baseline_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a"));
        assert!(!detector.observe(&text("a")));
    }

    #[test]
    fn new_content_is_a_change_once() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&Snapshot::Empty);
        assert!(detector.observe(&text("a")));
        assert!(!detector.observe(&text("a")));
        assert!(detector.observe(&text("b")));
    }

    #[test]
    fn empty_clipboard_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a"));
        assert!(!detector.observe(&Snapshot::Empty));
    }

    #[test]
    fn same_content_after_empty_is_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a"));
        assert!(!detector.observe(&Snapshot::Empty));
        assert!(detector.observe(&text("a")));
    }

    #[test]
    fn line_ending_only_difference_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a\nb"));
        assert!(!detector.observe(&text("a\r\nb")));
    }

    #[test]
    fn trailing_whitespace_only_difference_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("kidhr jaa rha hai ladle"));
        assert!(!detector.observe(&text("kidhr jaa rha hai ladle ")));
        assert!(!detector.observe(&text("kidhr jaa rha hai ladle\n")));
        assert!(detector.observe(&text("kidhr jaa rha hai ladle mt ja")));
    }

    #[test]
    fn image_changes_are_detected_by_pixels() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&image(0));
        assert!(!detector.observe(&image(0)));
        assert!(detector.observe(&image(1)));
    }

    #[test]
    fn text_and_image_are_distinct() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a"));
        assert!(detector.observe(&image(0)));
        assert!(detector.observe(&text("a")));
    }

    #[test]
    fn sensitive_content_is_reported_once_per_run() {
        let mut detector = ChangeDetector::default();
        detector.baseline(&text("a"));
        assert!(detector.observe(&SENSITIVE));
        assert!(!detector.observe(&SENSITIVE));
        assert!(detector.observe(&text("b")));
        assert!(detector.observe(&SENSITIVE));
    }
}
