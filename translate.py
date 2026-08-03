"""Translator — dịch text trước khi đọc (deep-translator / Google web).

Passthrough (trả nguyên văn) nếu tắt hoặc lỗi mạng.
"""


class Translator:
    def __init__(self, enabled: bool = False, source: str = "en", target: str = "vi"):
        self.enabled = enabled
        self.source = source
        self.target = target
        self._impl = None
        if enabled:
            try:
                from deep_translator import GoogleTranslator
                self._impl = GoogleTranslator(source=source, target=target)
            except Exception as e:
                print(f"[translate] khong khoi tao duoc, tat dich: {e}")
                self.enabled = False

    def translate(self, text: str) -> str:
        if not self.enabled or self._impl is None or not text.strip():
            return text
        try:
            out = self._impl.translate(text)
            return out or text
        except Exception as e:
            print(f"[translate] loi (tra nguyen van): {e}")
            return text
