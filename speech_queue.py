"""Hàng đợi đọc thuần logic: chống trùng + giới hạn (cap). Thread-safe.

Tách riêng để unit-test dễ (không phụ thuộc WinRT).
"""
import threading
from collections import deque
from typing import Optional


class SpeechQueue:
    def __init__(self, max_size: int = 3):
        self.max_size = max_size
        self._dq: deque[str] = deque()
        self._lock = threading.Lock()
        self._last_accepted = ""

    def enqueue(self, text: str) -> bool:
        """Đẩy câu vào hàng đợi. True nếu được nhận (không rỗng, không trùng câu vừa nhận)."""
        normalized = (text or "").strip()
        if not normalized:
            return False
        with self._lock:
            if normalized == self._last_accepted:
                return False
            self._last_accepted = normalized
            self._dq.append(normalized)
            while len(self._dq) > self.max_size:
                self._dq.popleft()  # bỏ câu cũ nhất
            return True

    def try_dequeue(self) -> Optional[str]:
        with self._lock:
            if self._dq:
                return self._dq.popleft()
        return None

    def clear(self) -> None:
        with self._lock:
            self._dq.clear()

    def reset_dedup(self) -> None:
        """Quên câu vừa nhận (dùng cho re-read chủ động)."""
        with self._lock:
            self._last_accepted = ""

    def __len__(self) -> int:
        with self._lock:
            return len(self._dq)
