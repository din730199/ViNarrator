"""ScreenCapture — chụp một vùng màn hình thành BGRA bytes (dùng mss).

LƯU Ý: mss yêu cầu khởi tạo & dùng trong CÙNG một thread.
=> tạo ScreenCapture() bên trong thread vòng lặp capture.
"""
import mss


class ScreenCapture:
    def __init__(self):
        self._sct = mss.mss()

    def grab_bgra(self, region: dict):
        mon = {
            "left": int(region["left"]),
            "top": int(region["top"]),
            "width": int(region["width"]),
            "height": int(region["height"]),
        }
        img = self._sct.grab(mon)
        return bytes(img.bgra), img.width, img.height
