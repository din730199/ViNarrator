"""Tiến trình con VieNeu — nói chuyện với ViNarrator qua stdin/stdout.

File này được nhúng vào exe (`include_str!`) và ghi ra đĩa lúc cài runtime.

Giao thức (một dòng = một lệnh):
    vào : <câu cần đọc>\\n
    ra  : "V <tên giọng>\\n" liệt kê giọng có sẵn, in hết TRƯỚC "READY"
          "READY\\n"       nạp xong model, chạy nóng xong, sẵn sàng nhận câu
          "C\\n"           vừa ghi xong một mẩu tiếng ra <out>/NNNN.wav
          "E\\n"           hết câu
          "X <lý do>\\n"   câu này hỏng (KHÔNG chết tiến trình)

Vì sao ghi ra FILE chứ không đẩy byte qua stdout: trộn nhị phân với văn bản trên stdout rất
dễ sai. Bên Rust tự đếm số thứ tự nên không phải đọc tên file — miễn nhiễm bảng mã (tên user
có thể là tiếng Việt).

Mỗi mẩu là một WAV hoàn chỉnh 16-bit; Rust xếp thẳng vào hàng đợi `waveOut` để nối liền mạch
(KHÔNG phải PlaySound — mở lại thiết bị mỗi mẩu sẽ chèn khe im vào giữa từ, xem wave_out.rs).
"""
import os
import sys
import wave

import numpy as np

OUT = sys.argv[1]
THREADS = int(sys.argv[2]) if len(sys.argv) > 2 else 2
GIONG = sys.argv[3] if len(sys.argv) > 3 else ""
SR = 48_000


def ghi_wav(path, x):
    # infer_stream trả float32 [-1,1] -> PCM 16-bit.
    pcm = (np.clip(x, -1.0, 1.0) * 32767.0).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def main():
    os.makedirs(OUT, exist_ok=True)
    from vieneu import Vieneu

    tts = Vieneu(backend="onnx", threads=THREADS)
    # Báo danh sách giọng THẬT của model thay vì để phía Rust chép cứng — model đổi danh
    # sách thì UI tự đúng theo.
    for ten in tts._preset_voices:
        print(f"V {ten}", flush=True)
    giong = GIONG if GIONG in tts._preset_voices else list(tts._preset_voices)[0]
    # Chạy nóng TRƯỚC khi báo READY: lượt đầu tốn ~1,5s dựng đồ thị, và lần chạy đầu tiên
    # của máy còn phải tải ~285 MB model. Câu thật của người dùng không nên gánh cái đó.
    for _ in tts.infer_stream("Xin chào", voice=giong, apply_watermark=False):
        pass
    print("READY", flush=True)

    seq = 0
    for line in sys.stdin:
        cau = line.strip()
        if not cau:
            continue
        try:
            for ch in tts.infer_stream(cau, voice=giong, apply_watermark=False):
                if ch is None or len(ch) == 0:
                    continue
                ghi_wav(os.path.join(OUT, f"{seq:04d}.wav"), np.asarray(ch).ravel())
                seq += 1
                print("C", flush=True)
            print("E", flush=True)
        except Exception as e:
            # Một câu hỏng KHÔNG được giết cả tiến trình — Rust lùi câu đó về giọng Windows.
            print(f"X {type(e).__name__}: {e}".replace("\n", " "), flush=True)


if __name__ == "__main__":
    main()
