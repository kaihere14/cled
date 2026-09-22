use crate::ClipboardContent;

/// Decides whether a clipboard observation is a new change.
///
/// Kept separate from the polling loop so the logic is testable without a real clipboard.
#[derive(Debug, Default)]
pub(crate) struct ChangeDetector {
    last: Option<u64>,
}

impl ChangeDetector {
    /// Records the initial clipboard state without reporting it as a change.
    pub(crate) fn baseline(&mut self, content: Option<&ClipboardContent>) {
        self.last = content.map(ClipboardContent::fingerprint);
    }

    /// Returns `true` if `content` differs from the previous observation.
    ///
    /// An empty or unsupported clipboard resets the state, so copying the same text again after
    /// copying something else still counts as a change.
    pub(crate) fn observe(&mut self, content: Option<&ClipboardContent>) -> bool {
        let current = content.map(ClipboardContent::fingerprint);
        let changed = current.is_some() && current != self.last;
        self.last = current;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ClipboardContent {
        ClipboardContent::text(s)
    }

    #[test]
    fn baseline_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(Some(&text("a")));
        assert!(!detector.observe(Some(&text("a"))));
    }

    #[test]
    fn new_content_is_a_change_once() {
        let mut detector = ChangeDetector::default();
        detector.baseline(None);
        assert!(detector.observe(Some(&text("a"))));
        assert!(!detector.observe(Some(&text("a"))));
        assert!(detector.observe(Some(&text("b"))));
    }

    #[test]
    fn empty_clipboard_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(Some(&text("a")));
        assert!(!detector.observe(None));
    }

    #[test]
    fn same_content_after_unsupported_content_is_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(Some(&text("a")));
        assert!(!detector.observe(None));
        assert!(detector.observe(Some(&text("a"))));
    }

    #[test]
    fn line_ending_only_difference_is_not_a_change() {
        let mut detector = ChangeDetector::default();
        detector.baseline(Some(&text("a\nb")));
        assert!(!detector.observe(Some(&text("a\r\nb"))));
    }
}
