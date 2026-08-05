//! Hàng đợi đọc: chống trùng + giới hạn (cap). Port từ speech_queue.py.
//! Thread-safe, không phụ thuộc WinRT nên unit-test được.

use std::collections::VecDeque;
use std::sync::Mutex;

pub struct SpeechQueue {
    max_size: usize,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    dq: VecDeque<String>,
    last_accepted: String,
}

impl SpeechQueue {
    pub fn new(max_size: usize) -> Self {
        Self { max_size, inner: Mutex::new(Inner::default()) }
    }

    /// Đẩy câu vào hàng đợi. true nếu được nhận (không rỗng, không trùng câu vừa nhận).
    pub fn enqueue(&self, text: &str) -> bool {
        let normalized = text.trim();
        if normalized.is_empty() {
            return false;
        }
        let mut g = self.inner.lock().unwrap();
        if normalized == g.last_accepted {
            return false;
        }
        g.last_accepted = normalized.to_string();
        g.dq.push_back(normalized.to_string());
        while g.dq.len() > self.max_size {
            g.dq.pop_front(); // bỏ câu cũ nhất
        }
        true
    }

    pub fn try_dequeue(&self) -> Option<String> {
        self.inner.lock().unwrap().dq.pop_front()
    }

    /// Không còn câu nào chờ đọc — dùng để biết lúc nào rảnh mà hâm nóng engine.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().dq.is_empty()
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap().dq.clear();
    }

    /// Quên câu vừa nhận (dùng cho re-read chủ động).
    pub fn reset_dedup(&self) {
        self.inner.lock().unwrap().last_accepted.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().dq.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rejected() {
        let q = SpeechQueue::new(3);
        assert!(!q.enqueue("   "));
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn consecutive_dup_rejected_after_dequeue() {
        let q = SpeechQueue::new(3);
        assert!(q.enqueue("a"));
        assert_eq!(q.try_dequeue().as_deref(), Some("a"));
        assert!(!q.enqueue("a")); // dedup giữ qua cả dequeue
    }

    #[test]
    fn trim_dup() {
        let q = SpeechQueue::new(3);
        assert!(q.enqueue(" a "));
        assert!(!q.enqueue("a"));
    }

    #[test]
    fn cap_drops_oldest() {
        let q = SpeechQueue::new(3);
        for t in ["a", "b", "c", "d"] {
            q.enqueue(t);
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.try_dequeue().as_deref(), Some("b")); // "a" bị bỏ
    }

    #[test]
    fn reset_dedup_allows_respeak() {
        let q = SpeechQueue::new(3);
        q.enqueue("a");
        q.try_dequeue();
        q.reset_dedup();
        assert!(q.enqueue("a"));
    }

    #[test]
    fn different_texts_accepted() {
        let q = SpeechQueue::new(3);
        assert!(q.enqueue("a"));
        assert!(q.enqueue("b"));
        assert_eq!(q.len(), 2);
    }
}
