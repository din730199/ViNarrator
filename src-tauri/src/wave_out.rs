//! Phát âm thanh LIỀN MẠCH bằng hàng đợi đệm `waveOut`.
//!
//! Vì sao không dùng `PlaySound` như hai engine kia: `PlaySound` mở lại thiết bị âm thanh ở
//! mỗi lần gọi. Với VieNeu (phát theo mẩu, 9 mẩu/câu) thì mỗi ranh giới mẩu thành một khe im
//! lặng — **đo được +71,6 ms trung bình, lớn nhất +110 ms, tổng +0,646s chèn vào câu 9,44s**.
//! Vì `infer_stream` cắt theo *frame* chứ không theo cụm từ, khe đó rơi vào **giữa từ** →
//! nghe rất giật. Cùng đống mẩu ấy ghép sẵn thành một file rồi phát một lần chỉ lệch +0,112s.
//!
//! `waveOut` nhận nhiều đệm xếp hàng và thiết bị tự nối chúng, không có khe. Mở thiết bị một
//! lần cho cả câu.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
    waveOutWrite, CALLBACK_EVENT, HWAVEOUT, WAVEFORMATEX, WAVEHDR, WAVE_FORMAT_PCM, WAVE_MAPPER,
    WHDR_DONE,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

/// Xếp trước tối đa ngần này đệm. Đủ để che lúc sinh mẩu chậm, mà không giữ quá nhiều tiếng
/// chưa phát (bấm DỪNG sẽ phải vứt hết chỗ đó đi).
const MAX_QUEUED: usize = 8;
/// Chờ tối đa mỗi nhịp rồi ngó lại cờ dừng — không chặn vô hạn trong `WaitForSingleObject`.
const WAIT_MS: u32 = 200;

struct Queued {
    /// Địa chỉ của header phải ĐỨNG YÊN trong lúc thiết bị đang cầm nó -> Box.
    hdr: Box<WAVEHDR>,
    /// Đệm dữ liệu cũng vậy: `hdr.lpData` trỏ vào đây.
    _data: Box<[u8]>,
}

pub struct WaveOut {
    hwo: HWAVEOUT,
    event: HANDLE,
    queued: Vec<Queued>,
}

impl WaveOut {
    pub fn open(sample_rate: u32) -> Result<Self, String> {
        let fmt = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM as u16,
            nChannels: 1,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate * 2,
            nBlockAlign: 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        // Event thủ công = false: hệ thống báo mỗi lần một đệm phát xong.
        let event = unsafe { CreateEventW(None, false, false, None) }
            .map_err(|e| format!("CreateEvent: {e}"))?;
        let mut hwo = HWAVEOUT::default();
        let r = unsafe {
            waveOutOpen(
                Some(&mut hwo),
                WAVE_MAPPER,
                &fmt,
                Some(event.0 as usize),
                None,
                CALLBACK_EVENT,
            )
        };
        if r != 0 {
            unsafe {
                let _ = CloseHandle(event);
            }
            return Err(format!("waveOutOpen lỗi {r}"));
        }
        Ok(Self { hwo, event, queued: Vec::new() })
    }

    /// Trả các đệm đã phát xong về cho hệ thống. Không gọi thì `queued` phình mãi.
    fn reap(&mut self) {
        let hwo = self.hwo;
        self.queued.retain_mut(|q| {
            if q.hdr.dwFlags & WHDR_DONE == 0 {
                return true;
            }
            unsafe {
                let _ = waveOutUnprepareHeader(
                    hwo,
                    &mut *q.hdr,
                    std::mem::size_of::<WAVEHDR>() as u32,
                );
            }
            false
        });
    }

    /// Xếp thêm một mẩu PCM 16-bit mono vào hàng đợi. Chặn khi hàng đã đầy (đó chính là cách
    /// giữ nhịp: sinh nhanh hơn phát nên chờ ở đây là bình thường).
    pub fn write(&mut self, pcm: &[u8], stop: &AtomicBool) -> Result<(), String> {
        if pcm.is_empty() {
            return Ok(());
        }
        self.reap();
        while self.queued.len() >= MAX_QUEUED {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            unsafe { WaitForSingleObject(self.event, WAIT_MS) };
            self.reap();
        }

        let mut data: Box<[u8]> = pcm.to_vec().into_boxed_slice();
        let mut hdr = Box::new(WAVEHDR {
            lpData: windows::core::PSTR(data.as_mut_ptr()),
            dwBufferLength: data.len() as u32,
            ..Default::default()
        });
        let sz = std::mem::size_of::<WAVEHDR>() as u32;
        let r = unsafe { waveOutPrepareHeader(self.hwo, &mut *hdr, sz) };
        if r != 0 {
            return Err(format!("waveOutPrepareHeader lỗi {r}"));
        }
        let r = unsafe { waveOutWrite(self.hwo, &mut *hdr, sz) };
        if r != 0 {
            unsafe {
                let _ = waveOutUnprepareHeader(self.hwo, &mut *hdr, sz);
            }
            return Err(format!("waveOutWrite lỗi {r}"));
        }
        self.queued.push(Queued { hdr, _data: data });
        Ok(())
    }

    /// Chờ phát hết chỗ đã xếp. Phải gọi trước khi sang câu sau, nếu không hai câu chồng tiếng.
    pub fn drain(&mut self, stop: &AtomicBool) {
        loop {
            self.reap();
            if self.queued.is_empty() {
                return;
            }
            if stop.load(Ordering::Relaxed) {
                self.reset();
                return;
            }
            if unsafe { WaitForSingleObject(self.event, WAIT_MS) } == WAIT_OBJECT_0 {
                continue;
            }
        }
    }

    /// Câm NGAY: bỏ hết đệm đang chờ. `waveOutReset` đánh dấu chúng đã xong để `reap` dọn được.
    pub fn reset(&mut self) {
        unsafe {
            let _ = waveOutReset(self.hwo);
        }
        self.reap();
    }
}

impl Drop for WaveOut {
    fn drop(&mut self) {
        self.reset();
        unsafe {
            let _ = waveOutClose(self.hwo);
            let _ = CloseHandle(self.event);
        }
    }
}

/// Cắt header WAV, trả (tần số, PCM). Duyệt chunk chứ không bê cứng 44 — cùng lý do với
/// `piper::apply_gain`.
pub fn split_wav(wav: &[u8]) -> Option<(u32, &[u8])> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let (mut i, mut sr) = (12usize, 0u32);
    while i + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[i + 4..i + 8].try_into().ok()?) as usize;
        match &wav[i..i + 4] {
            b"fmt " if i + 12 <= wav.len() => {
                sr = u32::from_le_bytes(wav[i + 12..i + 16].try_into().ok()?);
            }
            b"data" => {
                let end = (i + 8 + size).min(wav.len());
                return (sr > 0).then(|| (sr, &wav[i + 8..end]));
            }
            _ => {}
        }
        i += 8 + size + (size & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(sr: u32, mau: &[i16]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"RIFF\0\0\0\0WAVE");
        v.extend(b"fmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes()); // PCM
        v.extend(1u16.to_le_bytes()); // mono
        v.extend(sr.to_le_bytes());
        v.extend((sr * 2).to_le_bytes());
        v.extend(2u16.to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend((mau.len() as u32 * 2).to_le_bytes());
        for m in mau {
            v.extend(m.to_le_bytes());
        }
        v
    }

    #[test]
    fn tach_dung_tan_so_va_du_lieu() {
        let v = wav(48_000, &[-1000, 2000, 3000]);
        let (sr, pcm) = split_wav(&v).unwrap();
        assert_eq!(sr, 48_000);
        assert_eq!(pcm.len(), 6);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), -1000);
    }

    /// Chốt lý do duyệt chunk: có LIST chen vào thì data KHÔNG ở offset 44.
    #[test]
    fn bo_qua_chunk_la() {
        let mut v = wav(22_050, &[7, 8]);
        // chèn LIST (lẻ byte -> có đệm) ngay sau "WAVE"
        let mut chen = Vec::new();
        chen.extend(b"LIST");
        chen.extend(3u32.to_le_bytes());
        chen.extend([1, 2, 3, 0]);
        v.splice(12..12, chen);
        let (sr, pcm) = split_wav(&v).unwrap();
        assert_eq!(sr, 22_050);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), 7);
    }

    #[test]
    fn khong_phai_wav_thi_tra_none() {
        assert!(split_wav(b"khong phai wav").is_none());
        assert!(split_wav(&[]).is_none());
    }
}
