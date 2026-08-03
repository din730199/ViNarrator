"""Đọc/ghi cấu hình ViNarrator (config.json cạnh file này)."""
import json
import os

DEFAULT = {
    "region": {"left": 100, "top": 100, "width": 800, "height": 200},
    "exclude_region": None,          # vùng bỏ qua trong khung quét (hoặc None)
    "interval_ms": 1200,

    # Chống đọc trùng phụ đề cuộn
    "dedup_require_stable": True,    # đợi dòng ổn định 1 frame mới đọc (giảm đọc nửa câu)
    "min_line_len": 2,
    "fuzzy_threshold": 0.82,         # câu giống >= mức này coi là trùng (0..1; 1.0 = tắt fuzzy)

    # Chất lượng OCR (EasyOCR)
    "easyocr_min_confidence": 0.45,  # bỏ dòng OCR điểm tin cậy thấp (rác)
    "ocr_upscale": 2.0,              # phóng to ảnh trước khi OCR

    # Chế độ cấp cao: "translated" (Anh -> dịch -> đọc) | "plain" (đọc thuần tiếng Việt)
    "mode": "translated",

    # OCR
    "ocr_backend": "windows",        # "windows" | "easyocr"
    "ocr_language": "en-US",
    "easyocr_languages": ["vi"],

    # Dịch
    "translate": True,
    "translate_from": "en",
    "translate_to": "vi",

    # Giọng đọc
    "voice_name": "Microsoft An",
    "speed": 1.0,                    # bội số 0.5..2.0
    "volume": 100,                   # 0..100
}

PATH = os.path.join(os.path.dirname(__file__), "config.json")


def apply_mode(cfg: dict, mode: str) -> None:
    """Áp chế độ cấp cao -> set ocr_backend + ngôn ngữ + dịch tương ứng."""
    if mode == "plain":
        cfg["mode"] = "plain"
        cfg["ocr_backend"] = "easyocr"
        cfg["translate"] = False
    elif mode == "zh":  # Trung -> Việt
        cfg["mode"] = "zh"
        cfg["ocr_backend"] = "windows"
        cfg["ocr_language"] = "zh-Hans-CN"
        cfg["translate"] = True
        cfg["translate_from"] = "zh-CN"
        cfg["translate_to"] = "vi"
    else:  # "en" (hoặc legacy "translated"): Anh -> Việt
        cfg["mode"] = "en"
        cfg["ocr_backend"] = "windows"
        cfg["ocr_language"] = "en-US"
        cfg["translate"] = True
        cfg["translate_from"] = "en"
        cfg["translate_to"] = "vi"


def load() -> dict:
    if os.path.exists(PATH):
        try:
            with open(PATH, "r", encoding="utf-8") as f:
                data = json.load(f)
            merged = dict(DEFAULT)
            merged.update(data)
            return merged
        except Exception as e:
            print(f"[config] doc loi, dung mac dinh: {e}")
    return dict(DEFAULT)


def save(cfg: dict) -> None:
    with open(PATH, "w", encoding="utf-8") as f:
        json.dump(cfg, f, ensure_ascii=False, indent=2)
