"""LineDeduper — chống đọc trùng cho phụ đề cuộn/từng dòng.

Vấn đề: mỗi frame OCR bắt nguyên khối nhiều dòng chồng nhau (phụ đề cuộn),
chống trùng theo cả khối -> đổi 1 dòng là đọc lại hết.

Giải pháp:
  1. Tách theo DÒNG.
  2. Chỉ đọc dòng "ổn định" (xuất hiện ở frame trước) -> tránh đọc nửa câu đang hiện dần.
  3. Lọc trùng theo lịch sử dòng đã đọc + chứa-trong-nhau (vd 2 dòng gộp thành 1).
"""
import difflib
from collections import deque


class LineDeduper:
    def __init__(self, history: int = 40, min_len: int = 2, require_stable: bool = True,
                 fuzzy_threshold: float = 0.82):
        self.history = history
        self.min_len = min_len
        self.require_stable = require_stable
        self.fuzzy_threshold = fuzzy_threshold
        self._recent = deque(maxlen=history)   # key (đã chuẩn hoá) của các dòng đã đọc
        self._recent_set = set()
        self._prev_frame = set()                # key các dòng ở frame trước (để xét ổn định)

    @staticmethod
    def _key(s: str) -> str:
        return " ".join(s.split()).lower()

    def feed(self, ocr_text: str):
        """Trả về danh sách dòng (giữ nguyên gốc) MỚI cần đọc."""
        pairs = []
        for raw in (ocr_text or "").splitlines():
            original = raw.strip()
            key = self._key(original)
            if len(key) >= self.min_len:
                pairs.append((key, original))

        cur_keys = set(k for k, _ in pairs)
        out = []
        for key, original in pairs:
            stable = (not self.require_stable) or (key in self._prev_frame) or (key in self._recent_set)
            if not stable:
                continue
            if self._is_dup(key):
                continue
            out.append(original)
            self._recent.append(key)
            self._recent_set = set(self._recent)

        self._prev_frame = cur_keys
        return out

    def _is_dup(self, key: str) -> bool:
        if key in self._recent_set:
            return True
        # đã đọc khi nằm trong một dòng dài hơn
        for r in self._recent:
            if key in r:
                return True
        # dòng này gồm toàn nội dung đã đọc? (vd 2 dòng gộp thành 1 dòng dài)
        leftover = key
        for r in self._recent:
            if r and r in leftover:
                leftover = leftover.replace(r, " ")
        new_words = [w for w in leftover.split() if w]
        if len(new_words) <= 1:
            return True
        # gần giống dòng đã đọc (OCR sai vài ký tự) -> coi là trùng
        if self.fuzzy_threshold < 1.0:
            for r in self._recent:
                if difflib.SequenceMatcher(None, key, r).ratio() >= self.fuzzy_threshold:
                    return True
        return False

    def reset(self):
        self._recent.clear()
        self._recent_set.clear()
        self._prev_frame.clear()
