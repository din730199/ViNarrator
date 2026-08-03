"""Danh mục giọng đọc: Windows offline (winrt) + Piper offline + edge-tts online.

Mỗi giọng: (label hiển thị, engine, voice_id).
"""
import os

from winsdk.windows.media.speechsynthesis import SpeechSynthesizer

# Giọng edge-tts (online, miễn phí, neural). Thêm tuỳ ý.
EDGE_VOICES = [
    ("🌐 HoaiMy — Nữ (edge)", "vi-VN-HoaiMyNeural"),
    ("🌐 NamMinh — Nam (edge)", "vi-VN-NamMinhNeural"),
]

# Giọng Piper (OFFLINE, neural, nhanh). voice_id = tên file model trong models/.
PIPER_DIR = os.path.join(os.path.dirname(__file__), "models")
PIPER_VOICES = [
    ("🔌 Piper Vais — Nữ (offline, nhanh)", "vi_VN-vais1000-medium"),
]

# Giọng MMS (OFFLINE, qua transformers/torch). Model trong models/<id>/.
MMS_VOICES = [
    ("🔌 MMS — Nam (offline)", "mms-tts-vie"),
]


def _edge_available() -> bool:
    try:
        import edge_tts  # noqa: F401
        return True
    except Exception:
        return False


def _piper_available() -> bool:
    try:
        import piper  # noqa: F401
        return True
    except Exception:
        return False


def piper_model_path(voice_id: str) -> str:
    base = voice_id.split("#")[0]  # bỏ phần "#speaker_id" nếu có
    return os.path.join(PIPER_DIR, base + ".onnx")


def mms_model_path(voice_id: str = "mms-tts-vie") -> str:
    return os.path.join(PIPER_DIR, voice_id)


def _mms_available() -> bool:
    try:
        import transformers  # noqa: F401
        import torch  # noqa: F401
    except Exception:
        return False
    p = mms_model_path()
    return os.path.isfile(os.path.join(p, "model.safetensors")) and os.path.isfile(os.path.join(p, "config.json"))


def list_voices():
    out = []
    try:
        for v in SpeechSynthesizer.all_voices:
            if v.language.lower().startswith("vi"):  # chỉ giữ giọng Windows tiếng Việt
                out.append((f"{v.display_name} ({v.language})", "winrt", v.display_name))
    except Exception:
        pass
    if _piper_available():
        for (lbl, vid) in PIPER_VOICES:
            if os.path.exists(piper_model_path(vid)):
                out.append((lbl, "piper", vid))
    if _mms_available():
        out.extend((lbl, "mms", vid) for (lbl, vid) in MMS_VOICES)
    # edge (online) đã bỏ theo yêu cầu — chỉ dùng giọng offline.
    return out


def list_labels():
    return [lbl for (lbl, _, _) in list_voices()]


def resolve(name):
    """name (label hoặc display_name cũ) -> (engine, voice_id)."""
    vs = list_voices()
    for (lbl, engine, vid) in vs:
        if lbl == name:
            return engine, vid
    for (lbl, engine, vid) in vs:  # tương thích config cũ (chỉ lưu display_name)
        if engine == "winrt" and vid == name:
            return engine, vid
    for v in SpeechSynthesizer.all_voices:  # fallback: giọng tiếng Việt offline
        if v.language.lower().startswith("vi"):
            return "winrt", v.display_name
    return "winrt", ""


def match_label(name):
    """Tìm label khớp với giá trị đã lưu (cho việc set sẵn dropdown)."""
    labels = list_labels()
    for lbl in labels:
        if lbl == name:
            return lbl
    for lbl in labels:
        if name and name in lbl:
            return lbl
    return labels[0] if labels else ""
