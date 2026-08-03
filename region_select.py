"""Chọn vùng bằng cách kéo chuột (overlay). Dùng được standalone hoặc trong GUI.

select_region(master=None): nếu có master (CTk/Tk) -> dùng Toplevel; nếu không -> Tk riêng.
Trả về dict {left, top, width, height} hoặc None.
"""
import tkinter as tk


def select_region(master=None, title="Kéo chuột để chọn vùng  •  Esc để huỷ"):
    standalone = master is None
    root = tk.Tk() if standalone else tk.Toplevel(master)
    root.attributes("-fullscreen", True)
    root.attributes("-alpha", 0.25)
    root.attributes("-topmost", True)
    root.configure(bg="black")

    canvas = tk.Canvas(root, cursor="cross", bg="gray11", highlightthickness=0)
    canvas.pack(fill="both", expand=True)
    rect = canvas.create_rectangle(0, 0, 0, 0, outline="red", width=2)

    start = {}
    sel = {}

    def on_down(e):
        start["x"], start["y"] = e.x_root, e.y_root

    def on_move(e):
        if "x" in start:
            canvas.coords(rect, start["x"], start["y"], e.x_root, e.y_root)

    def on_up(e):
        if "x" not in start:
            return
        x1, y1, x2, y2 = start["x"], start["y"], e.x_root, e.y_root
        sel["left"] = int(min(x1, x2))
        sel["top"] = int(min(y1, y2))
        sel["width"] = int(abs(x2 - x1))
        sel["height"] = int(abs(y2 - y1))
        root.destroy()

    canvas.bind("<Button-1>", on_down)
    canvas.bind("<B1-Motion>", on_move)
    canvas.bind("<ButtonRelease-1>", on_up)
    root.bind("<Escape>", lambda e: root.destroy())

    tk.Label(root, text=title, bg="black", fg="white",
             font=("Segoe UI", 14)).place(relx=0.5, rely=0.04, anchor="n")

    if standalone:
        root.mainloop()
    else:
        root.grab_set()
        master.wait_window(root)

    if sel.get("width", 0) > 5 and sel.get("height", 0) > 5:
        return sel
    return None
