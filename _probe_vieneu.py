import sys
sys.stdout.reconfigure(encoding="utf-8", errors="replace")
import os
import time

import winsound
from vieneu import Vieneu

t0 = time.time()
tts = Vieneu(emotion="natural")
print("init:", round(time.time() - t0, 1), "s")

voices = tts.list_preset_voices()
print("VOICES:")
for v in voices:
    print("   ", v)

# tìm giọng nam (Phạm Tuyên / nam / male)
male = None
for item in voices:
    desc = str(item[0]) if isinstance(item, (list, tuple)) else str(item)
    if any(k in desc.lower() for k in ["tuyên", "tuyen", "nam", "male", "ông", "doctor"]):
        male = item
        break
if male is None:
    male = voices[1] if len(voices) > 1 else voices[0]
print("CHON NAM:", male)

vid = male[1] if isinstance(male, (list, tuple)) else male
voice_data = tts.get_preset_voice(vid)

tts.infer(text="xin chào", voice=voice_data)  # warmup
for s in ["Một đêm nọ, lão ngỏ lời hợp tác.",
          "Người anh hùng bước vào lâu đài tối tăm và tìm thấy thanh kiếm."]:
    t0 = time.time()
    audio = tts.infer(text=s, voice=voice_data)
    print("synth:", round(time.time() - t0, 2), "s |", s)

out = os.path.join(os.path.dirname(__file__), "_vn.wav")
tts.save(audio, out)
winsound.PlaySound(out, winsound.SND_FILENAME)
os.remove(out)
print("played")
