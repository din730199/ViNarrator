"""Narrator — bộ điều khiển: worker thread chụp -> (mask loại trừ) -> OCR -> (dịch) -> đọc.

Dùng chung cho GUI và CLI. Callbacks on_status(str) / on_text(orig, spoken) để cập nhật UI.
"""
import threading
import time

import numpy as np

import config as cfg_mod
from tts import TtsEngine
from ocr import make_ocr
from translate import Translator
from capture import ScreenCapture
from dedup import LineDeduper


def mask_exclusion(bgra: bytes, w: int, h: int, scan: dict, exclude) -> bytes:
    """Tô trắng vùng loại trừ (toạ độ màn hình) trong ảnh khung quét."""
    if not exclude:
        return bgra
    x0 = int(exclude["left"]) - int(scan["left"])
    y0 = int(exclude["top"]) - int(scan["top"])
    x1 = x0 + int(exclude["width"])
    y1 = y0 + int(exclude["height"])
    x0 = max(0, min(w, x0)); x1 = max(0, min(w, x1))
    y0 = max(0, min(h, y0)); y1 = max(0, min(h, y1))
    if x1 <= x0 or y1 <= y0:
        return bgra
    if len(bgra) != w * h * 4:
        return bgra
    arr = np.frombuffer(bgra, dtype=np.uint8).reshape(h, w, 4).copy()
    arr[y0:y1, x0:x1] = 255
    return arr.tobytes()


class Narrator:
    def __init__(self, cfg: dict, on_status=None, on_text=None):
        self.cfg = cfg
        self.on_status = on_status or (lambda s: None)
        self.on_text = on_text or (lambda o, s: None)
        self._tts = TtsEngine(cfg.get("voice_name", ""), cfg.get("speed", 1.0), cfg.get("volume", 100))
        self._lock = threading.Lock()
        self._running = False
        self._quit = False
        self._reload_ocr = True
        self._last_text = ""
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._worker.start()

    # ---------- điều khiển ----------
    @property
    def running(self) -> bool:
        with self._lock:
            return self._running

    def start(self):
        with self._lock:
            self._running = True

    def stop(self):
        with self._lock:
            self._running = False
        self._tts.stop_playback()

    def toggle(self) -> bool:
        with self._lock:
            self._running = not self._running
            r = self._running
        if not r:
            self._tts.stop_playback()
        return r

    def reread(self):
        with self._lock:
            t = self._last_text
        if t:
            self._tts.speak_now(t)

    def set_mode(self, mode: str):
        with self._lock:
            cfg_mod.apply_mode(self.cfg, mode)
            self._reload_ocr = True

    def set_voice(self, name: str):
        with self._lock:
            self.cfg["voice_name"] = name
        self._tts.set_params(voice_name=name)

    def set_speed(self, speed: float):
        with self._lock:
            self.cfg["speed"] = speed
        self._tts.set_params(speed=speed)

    def set_volume(self, volume: int):
        with self._lock:
            self.cfg["volume"] = volume
        self._tts.set_params(volume=volume)

    def set_region(self, region: dict):
        with self._lock:
            self.cfg["region"] = region

    def set_exclude(self, region):
        with self._lock:
            self.cfg["exclude_region"] = region

    def shutdown(self):
        with self._lock:
            self._quit = True
        self._tts.shutdown()

    def join(self):
        self._worker.join()

    # ---------- worker ----------
    def _run(self):
        cap = ScreenCapture()  # mss: tạo trong thread này
        ocr = None
        translator = None
        deduper = LineDeduper(
            require_stable=self.cfg.get("dedup_require_stable", True),
            min_len=self.cfg.get("min_line_len", 2),
            fuzzy_threshold=self.cfg.get("fuzzy_threshold", 0.82),
        )
        while True:
            with self._lock:
                if self._quit:
                    return
                running = self._running
                reload_ocr = self._reload_ocr
                self._reload_ocr = False
                region = dict(self.cfg["region"])
                exclude = self.cfg.get("exclude_region")
                interval = self.cfg.get("interval_ms", 1200)

            if reload_ocr:
                self.on_status("Đang khởi tạo OCR...")
                try:
                    ocr = make_ocr(self.cfg)
                    translator = Translator(
                        bool(self.cfg.get("translate", False)),
                        self.cfg.get("translate_from", "en"),
                        self.cfg.get("translate_to", "vi"),
                    )
                    self.on_status(f"Sẵn sàng — {ocr.info}, dịch {'ON' if translator.enabled else 'OFF'}")
                except Exception as e:
                    self.on_status(f"Lỗi khởi tạo OCR: {e}")
                    ocr = None
                deduper.reset()

            if not running or ocr is None:
                time.sleep(0.1)
                continue

            t0 = time.monotonic()
            try:
                bgra, w, h = cap.grab_bgra(region)
                bgra = mask_exclusion(bgra, w, h, region, exclude)
                text = ocr.recognize_bgra(bgra, w, h)
                for line in deduper.feed(text):
                    spoken = translator.translate(line)
                    with self._lock:
                        self._last_text = spoken
                    self.on_text(line, spoken)
                    self._tts.speak(spoken)
            except Exception as e:
                self.on_status(f"Lỗi: {e}")

            # giữ nhịp ~interval (trừ thời gian OCR đã tốn) -> không cộng dồn -> nhạy hơn
            elapsed = time.monotonic() - t0
            time.sleep(max(0.02, interval / 1000.0 - elapsed))
