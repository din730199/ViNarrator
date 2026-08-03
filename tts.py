"""TtsEngine — đọc tuần tự, chọn engine theo giọng:
  - winrt (offline): Windows.Media.SpeechSynthesis -> WAV -> winsound.
  - edge  (online):  edge-tts -> mp3 -> winsdk MediaPlayer (blocking).

Worker thread + phát blocking => đọc tuần tự. Chống trùng + cap qua SpeechQueue.
"""
import asyncio
import io
import logging
import os
import tempfile
import threading
import wave
import winsound

logging.getLogger("piper").setLevel(logging.ERROR)  # tắt cảnh báo "Missing phoneme"

from winsdk.windows.media.speechsynthesis import SpeechSynthesizer
from winsdk.windows.storage.streams import DataReader

import voices as voice_catalog
from speech_queue import SpeechQueue


def list_vietnamese_voices():
    return [v.display_name for v in SpeechSynthesizer.all_voices
            if v.language.lower().startswith("vi")]


class TtsEngine:
    def __init__(self, voice_name: str = "", speed: float = 1.0, volume: int = 100, max_queue: int = 3):
        self._queue = SpeechQueue(max_size=max_queue)
        self._voice_name = voice_name
        self._speed = speed
        self._volume = volume
        self._piper_voices = {}  # cache PiperVoice theo voice_id
        self._mms = None          # cache (model, tokenizer, sample_rate)
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._worker.start()

    # ---------- public ----------
    def speak(self, text: str) -> None:
        if self._queue.enqueue(text):
            self._wake.set()

    def speak_now(self, text: str) -> None:
        self._queue.reset_dedup()
        self.speak(text)

    def stop_playback(self) -> None:
        self._queue.clear()
        try:
            winsound.PlaySound(None, winsound.SND_PURGE)
        except Exception:
            pass

    def shutdown(self) -> None:
        self._stop.set()
        self._wake.set()

    def set_params(self, voice_name=None, speed=None, volume=None) -> None:
        if voice_name is not None:
            self._voice_name = voice_name
        if speed is not None:
            self._speed = speed
        if volume is not None:
            self._volume = volume

    # ---------- worker ----------
    def _run(self) -> None:
        while not self._stop.is_set():
            text = self._queue.try_dequeue()
            if text is None:
                self._wake.wait()
                self._wake.clear()
                continue
            try:
                engine, vid = voice_catalog.resolve(self._voice_name)
                if engine == "edge":
                    self._speak_edge(text, vid)
                elif engine == "piper":
                    self._speak_piper(text, vid)
                elif engine == "mms":
                    self._speak_mms(text, vid)
                else:
                    wav = asyncio.run(self._synth_winrt(text, vid))
                    winsound.PlaySound(wav, winsound.SND_MEMORY)
            except Exception as e:
                print(f"[TTS] loi: {e}")

    # ---------- winrt (offline) ----------
    async def _synth_winrt(self, text: str, display_name: str) -> bytes:
        synth = SpeechSynthesizer()
        if display_name:
            for v in SpeechSynthesizer.all_voices:
                if v.display_name == display_name:
                    synth.voice = v
                    break
        synth.options.speaking_rate = max(0.5, min(6.0, float(self._speed)))
        synth.options.audio_volume = max(0, min(100, self._volume)) / 100.0

        stream = await synth.synthesize_text_to_stream_async(text)
        size = stream.size
        reader = DataReader(stream)
        await reader.load_async(size)
        buf = bytearray(size)
        reader.read_bytes(buf)
        return bytes(buf)

    # ---------- piper (offline, neural, nhanh) ----------
    def _get_piper(self, voice_id: str):
        if voice_id not in self._piper_voices:
            from piper import PiperVoice
            path = voice_catalog.piper_model_path(voice_id)
            v = PiperVoice.load(path)
            # warmup để câu đầu không bị chậm
            try:
                b = io.BytesIO()
                with wave.open(b, "wb") as wf:
                    v.synthesize_wav("xin chào", wf)
            except Exception:
                pass
            self._piper_voices[voice_id] = v
        return self._piper_voices[voice_id]

    def _piper_syn_config(self, speaker_id=None):
        try:
            from piper import SynthesisConfig
            kwargs = dict(
                length_scale=1.0 / max(0.3, float(self._speed)),  # speed cao -> length_scale nhỏ -> nhanh
                volume=max(0.0, min(1.0, self._volume / 100.0)),
            )
            if speaker_id is not None:
                kwargs["speaker_id"] = speaker_id
            return SynthesisConfig(**kwargs)
        except Exception:
            return None

    def _speak_piper(self, text: str, voice_id: str) -> None:
        voice = self._get_piper(voice_id)
        _, _, sp = voice_id.partition("#")  # "model#speaker_id"
        speaker_id = int(sp) if sp.isdigit() else None
        cfg = self._piper_syn_config(speaker_id)
        buf = io.BytesIO()
        with wave.open(buf, "wb") as wf:
            try:
                if cfg is not None:
                    voice.synthesize_wav(text, wf, syn_config=cfg)
                else:
                    voice.synthesize_wav(text, wf)
            except TypeError:
                voice.synthesize_wav(text, wf)
        winsound.PlaySound(buf.getvalue(), winsound.SND_MEMORY)

    # ---------- mms (offline, transformers/torch) ----------
    def _get_mms(self):
        if self._mms is None:
            import torch  # noqa: F401
            from transformers import VitsModel, AutoTokenizer
            path = voice_catalog.mms_model_path()
            model = VitsModel.from_pretrained(path)
            model.eval()
            tok = AutoTokenizer.from_pretrained(path)
            self._mms = (model, tok, int(model.config.sampling_rate))
            try:
                self._mms_synth("xin chào")  # warmup
            except Exception:
                pass
        return self._mms

    def _mms_synth(self, text: str) -> bytes:
        import numpy as np
        import torch
        model, tok, sr = self._mms
        # đọc nhanh hơn theo thanh TỐC ĐỘ — dùng speaking_rate gốc (sạch, KHÔNG rè)
        try:
            model.speaking_rate = max(0.5, min(2.0, float(self._speed)))
        except Exception:
            pass
        inputs = tok(text, return_tensors="pt")
        with torch.inference_mode():
            wav = model(**inputs).waveform.squeeze().cpu().numpy().astype(np.float32)
        # chuẩn hoá biên độ cho to (MMS xuất ra hơi nhỏ)
        peak = float(np.max(np.abs(wav))) if wav.size else 0.0
        if peak > 1e-4:
            wav = wav / peak * 0.97
        vol = max(0.0, min(1.0, self._volume / 100.0))
        pcm = (np.clip(wav, -1, 1) * 32767 * vol).astype("<i2")
        buf = io.BytesIO()
        with wave.open(buf, "wb") as wf:
            wf.setnchannels(1)
            wf.setsampwidth(2)
            wf.setframerate(sr)
            wf.writeframes(pcm.tobytes())
        return buf.getvalue()

    def _speak_mms(self, text: str, voice_id: str) -> None:
        self._get_mms()
        winsound.PlaySound(self._mms_synth(text), winsound.SND_MEMORY)

    # ---------- edge (online) ----------
    def _speak_edge(self, text: str, voice_id: str) -> None:
        import edge_tts
        rate = f"{int(round((self._speed - 1) * 100)):+d}%"  # 1.3 -> +30%
        path = os.path.join(tempfile.gettempdir(), f"_vinarr_{threading.get_ident()}.mp3")

        async def gen():
            await edge_tts.Communicate(text, voice_id, rate=rate).save(path)

        asyncio.run(gen())
        self._play_mp3_blocking(path)
        try:
            os.remove(path)
        except Exception:
            pass

    def _play_mp3_blocking(self, path: str) -> None:
        from winsdk.windows.media.playback import MediaPlayer
        from winsdk.windows.media.core import MediaSource
        from winsdk.windows.foundation import Uri

        player = MediaPlayer()
        done = threading.Event()
        player.add_media_ended(lambda s, e: done.set())
        player.add_media_failed(lambda s, e: done.set())
        player.source = MediaSource.create_from_uri(Uri("file:///" + path.replace("\\", "/")))
        player.volume = max(0.0, min(1.0, self._volume / 100.0))
        player.play()
        done.wait(30)
        try:
            player.close()
        except Exception:
            pass
