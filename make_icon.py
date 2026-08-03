"""Tạo icon vinarrator.ico cho shortcut Desktop (chạy 1 lần)."""
import os

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
SIZE = 256

img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# nền tím bo góc (đồng bộ màu GUI #7c3aed)
d.rounded_rectangle([8, 8, SIZE - 8, SIZE - 8], radius=48, fill=(124, 58, 237, 255))

# chữ "VN" trắng đậm ở giữa
try:
    font = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 124)
except Exception:
    font = ImageFont.load_default()
text = "VN"
bbox = d.textbbox((0, 0), text, font=font)
tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
d.text(((SIZE - tw) / 2 - bbox[0], (SIZE - th) / 2 - bbox[1]),
       text, font=font, fill=(255, 255, 255, 255))

ico = os.path.join(HERE, "vinarrator.ico")
img.save(ico, sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("OK ->", ico)
