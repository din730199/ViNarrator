"""ViNarrator — CLI / headless. (GUI: chạy `python gui.py`)

  python app.py                 # realtime headless + hotkey
  python app.py --gui           # mở giao diện
  python app.py --select-region # chọn vùng quét rồi lưu
  python app.py --once          # chụp+OCR+(dịch)+đọc 1 lần (test)
  python app.py --list-voices | --ocr-langs | --info

Hotkey (headless): Ctrl+Alt+S bật/tắt | Ctrl+Alt+R đọc lại | Ctrl+Alt+Q thoát
"""
import argparse
import sys
import time

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

import config as cfg_mod
from tts import TtsEngine, list_vietnamese_voices
from ocr import make_ocr, windows_available_languages
from translate import Translator
from capture import ScreenCapture
from narrator import Narrator


def run_headless(cfg):
    nar = Narrator(cfg,
                   on_status=lambda s: print("[status]", s),
                   on_text=lambda o, s: print("→", s))
    import keyboard
    keyboard.add_hotkey("ctrl+alt+s", lambda: print("DOC:", "ON" if nar.toggle() else "OFF"))
    keyboard.add_hotkey("ctrl+alt+r", nar.reread)
    keyboard.add_hotkey("ctrl+alt+q", nar.shutdown)
    print("San sang.  Ctrl+Alt+S bat/tat doc | Ctrl+Alt+R doc lai | Ctrl+Alt+Q thoat")
    nar.join()
    print("Thoat.")


def run_once(cfg):
    tts = TtsEngine(cfg.get("voice_name", ""), cfg.get("speed", 1.0), cfg.get("volume", 100))
    cap = ScreenCapture()
    ocr = make_ocr(cfg)
    translator = Translator(bool(cfg.get("translate", False)),
                            cfg.get("translate_from", "en"), cfg.get("translate_to", "vi"))
    print(f"[OCR] {ocr.info} | dich: {'ON' if translator.enabled else 'OFF'}")
    bgra, w, h = cap.grab_bgra(cfg["region"])
    text = ocr.recognize_bgra(bgra, w, h).strip()
    print("OCR :", repr(text))
    spoken = translator.translate(text)
    print("Doc :", repr(spoken))
    if spoken.strip():
        tts.speak(spoken)
        time.sleep(8)
    tts.shutdown()


def main():
    parser = argparse.ArgumentParser(description="ViNarrator - thuyet minh game tieng Viet")
    parser.add_argument("--gui", action="store_true")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--list-voices", action="store_true")
    parser.add_argument("--ocr-langs", action="store_true")
    parser.add_argument("--select-region", action="store_true")
    parser.add_argument("--info", action="store_true")
    args = parser.parse_args()

    cfg = cfg_mod.load()

    if args.gui:
        import gui
        gui.main()
        return
    if args.list_voices:
        import voices as voice_catalog
        for lbl in voice_catalog.list_labels():
            print(" -", lbl)
        return
    if args.ocr_langs:
        print("Windows OCR languages:", windows_available_languages())
        return
    if args.info:
        import json
        print(json.dumps(cfg, ensure_ascii=False, indent=2))
        return
    if args.select_region:
        from region_select import select_region
        r = select_region()
        if r:
            cfg["region"] = r
            cfg_mod.save(cfg)
            print("Da luu vung:", r)
        else:
            print("Huy chon vung.")
        return
    if args.once:
        run_once(cfg)
        return

    run_headless(cfg)


if __name__ == "__main__":
    main()
