"""Launcher KHÔNG cửa sổ console cho ViNarrator (mở bằng pythonw.exe).

Dùng cho shortcut Desktop. pythonw đặt sys.stdout/stderr = None nên mọi print()
sẽ lỗi -> đổ log ra file vinarrator.log (ghi đè mỗi lần mở) cho an toàn + dễ debug.
"""
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))

# Đảm bảo import được các module cạnh file (gui, narrator, ...) dù mở từ đâu.
if _HERE not in sys.path:
    sys.path.insert(0, _HERE)

# Chuyển stdout/stderr ra file log (pythonw không có console).
try:
    _f = open(os.path.join(_HERE, "vinarrator.log"), "w", encoding="utf-8", errors="replace")
    sys.stdout = _f
    sys.stderr = _f
except Exception:
    pass

import gui

gui.main()
