"""OCR backends + factory.

- WindowsOcrBackend: WinRT Windows.Media.Ocr (nhanh, en/zh... — KHÔNG đọc được tiếng Việt).
- EasyOcrBackend: EasyOCR (đọc được tiếng Việt, nặng hơn, offline sau khi tải model).

Cùng interface: recognize_bgra(bgra: bytes, w: int, h: int) -> str
"""
import asyncio


# ----------------------------- Windows OCR -----------------------------
class WindowsOcrBackend:
    def __init__(self, language_tag: str = "en-US"):
        from winsdk.windows.media.ocr import OcrEngine
        from winsdk.windows.globalization import Language
        self._OcrEngine = OcrEngine
        eng = OcrEngine.try_create_from_language(Language(language_tag))
        if eng is None:
            eng = OcrEngine.try_create_from_user_profile_languages()
        self._engine = eng

    @property
    def available(self) -> bool:
        return self._engine is not None

    @property
    def info(self) -> str:
        if self._engine is None:
            return "windows(KHONG CO engine)"
        return f"windows({self._engine.recognizer_language.language_tag})"

    def recognize_bgra(self, bgra: bytes, width: int, height: int) -> str:
        if self._engine is None:
            return ""
        return asyncio.run(self._recognize(bgra, width, height))

    async def _recognize(self, bgra: bytes, width: int, height: int) -> str:
        from winsdk.windows.graphics.imaging import SoftwareBitmap, BitmapPixelFormat
        from winsdk.windows.security.cryptography import CryptographicBuffer
        buffer = CryptographicBuffer.create_from_byte_array(bgra)
        bmp = SoftwareBitmap.create_copy_from_buffer(
            buffer, BitmapPixelFormat.BGRA8, width, height
        )
        result = await self._engine.recognize_async(bmp)
        return result.text


def windows_available_languages():
    from winsdk.windows.media.ocr import OcrEngine
    return [l.language_tag for l in OcrEngine.available_recognizer_languages]


# ----------------------------- EasyOCR -----------------------------
class EasyOcrBackend:
    def __init__(self, languages=None, min_confidence: float = 0.45, upscale: float = 2.0):
        self.languages = languages or ["vi"]
        self.min_confidence = min_confidence
        self.upscale = upscale
        self._reader = None
        self._np = None
        self._cv2 = None
        try:
            import easyocr
            import numpy as np
            self._np = np
            try:
                import cv2
                self._cv2 = cv2
            except Exception:
                self._cv2 = None
            print(f"[EasyOCR] dang tai model {self.languages} (lan dau co the lau)...")
            self._reader = easyocr.Reader(self.languages, gpu=False)
            print("[EasyOCR] san sang.")
        except Exception as e:
            print(f"[EasyOCR] khong khoi tao duoc: {e}")
            self._reader = None

    @property
    def available(self) -> bool:
        return self._reader is not None

    @property
    def info(self) -> str:
        return f"easyocr({','.join(self.languages)})" + ("" if self._reader else " [LOI]")

    def recognize_bgra(self, bgra: bytes, width: int, height: int) -> str:
        if self._reader is None:
            return ""
        np = self._np
        if len(bgra) != width * height * 4:
            return ""
        arr = np.frombuffer(bgra, dtype=np.uint8).reshape(height, width, 4)
        bgr = arr[:, :, :3]  # bỏ alpha; EasyOCR (cv2) nhận BGR

        # phóng to để chữ nhỏ rõ hơn -> OCR chính xác hơn
        if self._cv2 is not None and self.upscale and self.upscale != 1.0:
            bgr = self._cv2.resize(bgr, None, fx=self.upscale, fy=self.upscale,
                                   interpolation=self._cv2.INTER_CUBIC)

        # detail=1 -> có confidence; lọc bỏ dòng rác điểm thấp
        results = self._reader.readtext(bgr, detail=1, paragraph=False)
        lines = []
        for item in results:
            try:
                _, text, conf = item
            except Exception:
                continue
            if text and text.strip() and conf >= self.min_confidence:
                lines.append(text.strip())
        return "\n".join(lines)


# ----------------------------- factory -----------------------------
def make_ocr(cfg: dict):
    backend = str(cfg.get("ocr_backend", "windows")).lower()
    if backend == "easyocr":
        return EasyOcrBackend(
            cfg.get("easyocr_languages", ["vi"]),
            min_confidence=cfg.get("easyocr_min_confidence", 0.45),
            upscale=cfg.get("ocr_upscale", 2.0),
        )
    return WindowsOcrBackend(cfg.get("ocr_language", "en-US"))
