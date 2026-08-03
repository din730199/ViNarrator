"""ViNarrator GUI — giao diện dark (customtkinter) giống tool thuyết minh.

Chạy:  python gui.py
"""
import queue
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

import customtkinter as ctk

import config as cfg_mod
from narrator import Narrator
import voices as voice_catalog
from region_select import select_region

ctk.set_appearance_mode("dark")
ctk.set_default_color_theme("dark-blue")

PURPLE = "#7c3aed"
PURPLE_H = "#6d28d9"
RED = "#b91c1c"
RED_H = "#991b1b"
GREEN = "#16a34a"
GREEN_H = "#15803d"

MODE_LABELS = {
    "Đọc bản dịch (Anh→Việt)": "en",
    "Đọc bản dịch (Trung→Việt)": "zh",
    "Đọc thuần (Việt)": "plain",
}
MODE_INV = {v: k for k, v in MODE_LABELS.items()}


class App(ctk.CTk):
    def __init__(self):
        super().__init__()
        self.title("ViNarrator — Thuyết minh game")
        self.geometry("440x600")
        self.minsize(400, 560)

        self.cfg = cfg_mod.load()
        self._ui_queue = queue.Queue()
        self.narrator = Narrator(self.cfg, on_status=self._status_async, on_text=self._text_async)
        self._build()
        self.after(100, self._poll_ui)  # poll trên main thread

    # ---------------- UI ----------------
    def _build(self):
        lblpack = {"fill": "x", "padx": 18, "pady": (12, 2)}

        self.btn_toggle = ctk.CTkButton(self, text="▶  BẮT ĐẦU ĐỌC", height=46,
                                        font=("Segoe UI", 15, "bold"),
                                        fg_color=GREEN, hover_color=GREEN_H, command=self._toggle)
        self.btn_toggle.pack(fill="x", padx=18, pady=(16, 6))

        row = ctk.CTkFrame(self, fg_color="transparent")
        row.pack(fill="x", padx=18, pady=4)
        ctk.CTkButton(row, text="SỬA VÙNG QUÉT", fg_color=PURPLE, hover_color=PURPLE_H,
                      command=self._pick_scan).pack(side="left", expand=True, fill="x", padx=(0, 4))
        ctk.CTkButton(row, text="VÙNG LOẠI TRỪ", fg_color=RED, hover_color=RED_H,
                      command=self._pick_exclude).pack(side="left", expand=True, fill="x", padx=(4, 0))

        ctk.CTkLabel(self, text="CHẾ ĐỘ", anchor="w").pack(**lblpack)
        self.mode = ctk.CTkOptionMenu(self, values=list(MODE_LABELS.keys()),
                                      fg_color=PURPLE, button_color=PURPLE_H, command=self._set_mode)
        _m = self.cfg.get("mode", "en")
        if _m == "translated":
            _m = "en"
        self.mode.set(MODE_INV.get(_m, "Đọc bản dịch (Anh→Việt)"))
        self.mode.pack(fill="x", padx=18)

        ctk.CTkLabel(self, text="GIỌNG ĐỌC", anchor="w").pack(**lblpack)
        labels = voice_catalog.list_labels() or ["(chưa có giọng)"]
        self.voice = ctk.CTkOptionMenu(self, values=labels, fg_color=PURPLE,
                                       button_color=PURPLE_H, command=self._set_voice)
        self.voice.set(voice_catalog.match_label(self.cfg.get("voice_name", "")))
        self.voice.pack(fill="x", padx=18)

        self.lbl_vol = ctk.CTkLabel(self, text="ÂM LƯỢNG", anchor="w")
        self.lbl_vol.pack(**lblpack)
        self.vol = ctk.CTkSlider(self, from_=0, to=100, command=self._set_vol)
        self.vol.set(self.cfg.get("volume", 100))
        self.vol.pack(fill="x", padx=18)

        self.lbl_speed = ctk.CTkLabel(self, text="TỐC ĐỘ", anchor="w")
        self.lbl_speed.pack(**lblpack)
        self.speed = ctk.CTkSlider(self, from_=0.5, to=2.0, command=self._set_speed)
        self.speed.set(self.cfg.get("speed", 1.0))
        self.speed.pack(fill="x", padx=18)

        self.status = ctk.CTkLabel(self, text="", anchor="w", text_color="#9ca3af")
        self.status.pack(fill="x", padx=18, pady=(14, 2))
        self.txt = ctk.CTkTextbox(self, height=110)
        self.txt.pack(fill="both", expand=True, padx=18, pady=(0, 16))

        self._refresh_labels()
        self.protocol("WM_DELETE_WINDOW", self._on_close)

    def _refresh_labels(self):
        self.lbl_vol.configure(text=f"ÂM LƯỢNG   {int(self.vol.get())}%")
        self.lbl_speed.configure(text=f"TỐC ĐỘ   {self.speed.get():.1f}x")

    # ---------------- handlers ----------------
    def _toggle(self):
        on = self.narrator.toggle()
        if on:
            self.btn_toggle.configure(text="⏸  DỪNG ĐỌC", fg_color=RED, hover_color=RED_H)
        else:
            self.btn_toggle.configure(text="▶  BẮT ĐẦU ĐỌC", fg_color=GREEN, hover_color=GREEN_H)

    def _set_mode(self, label):
        self.narrator.set_mode(MODE_LABELS.get(label, "translated"))
        cfg_mod.save(self.cfg)

    def _set_voice(self, name):
        self.narrator.set_voice(name)
        cfg_mod.save(self.cfg)

    def _set_vol(self, _=None):
        v = int(self.vol.get())
        self.narrator.set_volume(v)
        self._refresh_labels()
        cfg_mod.save(self.cfg)

    def _set_speed(self, _=None):
        s = round(self.speed.get(), 1)
        self.narrator.set_speed(s)
        self._refresh_labels()
        cfg_mod.save(self.cfg)

    def _pick_scan(self):
        self.withdraw()
        r = select_region(self, "Kéo chọn VÙNG QUÉT (phụ đề/thoại)  •  Esc huỷ")
        self.deiconify()
        if r:
            self.narrator.set_region(r)
            cfg_mod.save(self.cfg)
            self._status(f"Vùng quét: {r['width']}×{r['height']}")

    def _pick_exclude(self):
        self.withdraw()
        r = select_region(self, "Kéo chọn VÙNG LOẠI TRỪ (bỏ qua)  •  Esc huỷ")
        self.deiconify()
        if r:
            self.narrator.set_exclude(r)
            cfg_mod.save(self.cfg)
            self._status(f"Vùng loại trừ: {r['width']}×{r['height']}")

    def _on_close(self):
        self.narrator.shutdown()
        self.destroy()

    # ---------------- callbacks (từ worker thread) ----------------
    def _status_async(self, s):
        self._ui_queue.put(("status", s))

    def _text_async(self, orig, spoken):
        self._ui_queue.put(("text", spoken))

    def _poll_ui(self):
        try:
            while True:
                item = self._ui_queue.get_nowait()
                if item[0] == "status":
                    self.status.configure(text=item[1])
                elif item[0] == "text":
                    self.txt.insert("end", item[1] + "\n")
                    self.txt.see("end")
        except queue.Empty:
            pass
        self.after(100, self._poll_ui)

    def _status(self, s):
        self.status.configure(text=s)


def main():
    App().mainloop()


if __name__ == "__main__":
    main()
