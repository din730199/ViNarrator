import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Config, Spoken } from '../types';

// Hết thoại bao lâu thì ẩn hộp đi: cửa sổ vẫn phủ vùng đó trên game, ẩn cho đỡ vướng mắt.
const IDLE_MS = 8000;
// Lúc chơi (chuột xuyên qua) chỉ hiện ngần này câu: nhiều hơn thì hộp che mất cảnh game.
const GON = 3;
// Giữ lại để lăn chuột xem lại khi bật chế độ tương tác.
const GIU = 50;
// Câu mới nhất -> cũ dần: giống khung lời thoại ở cửa sổ chính. Cũ hơn nữa dùng mức cuối.
const MAU = ['#f4b942', '#b9b8b2', '#8a8f9a'];
const CO = [22, 17, 15];

export default function Captions() {
  const [cues, setCues] = useState<Spoken[]>([]);
  const [visible, setVisible] = useState(false);
  // Bật = cửa sổ nhận chuột: kéo để dời, lăn để xem lại. Tắt = chuột xuyên qua vào game.
  const [movable, setMovable] = useState(false);
  const hop = useRef<HTMLDivElement>(null);
  // Người dùng đang lăn lên đọc câu cũ thì câu mới tới KHÔNG được giật về cuối.
  const dangXemLai = useRef(false);

  useEffect(() => {
    void invoke<Config>('get_config').then((c) => setMovable(c.captions_movable));
    const un = [
      listen<Spoken>('text', (e) => {
        setCues((l) => [...l, e.payload].slice(-GIU));
        setVisible(true);
      }),
      listen<boolean>('captions_state', (e) => setMovable(e.payload)),
    ];
    return () => {
      void Promise.all(un).then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  useEffect(() => {
    if (!visible) return;
    const t = setTimeout(() => setVisible(false), IDLE_MS);
    return () => clearTimeout(t);
  }, [cues, visible]);

  // Luôn bám câu mới nhất, trừ khi đang lăn lên xem lại. Tắt tương tác là về cuối ngay.
  useEffect(() => {
    if (!movable) dangXemLai.current = false;
    const h = hop.current;
    if (h && !dangXemLai.current) h.scrollTop = h.scrollHeight;
  }, [cues, movable]);

  const hien = movable ? cues : cues.slice(-GON);
  const show = visible || movable;
  return (
    <div
      style={{ height: '100%', display: 'flex', alignItems: 'flex-end', justifyContent: 'center' }}
    >
      <style>{`.hop::-webkit-scrollbar{width:6px}.hop::-webkit-scrollbar-thumb{background:#3a3f4a;border-radius:3px}`}</style>
      <div
        ref={hop}
        className="hop"
        onMouseDown={(e) => {
          // Bấm trúng thanh cuộn (mép phải) thì để cuộn, không kéo cửa sổ.
          const r = e.currentTarget.getBoundingClientRect();
          if (movable && e.clientX < r.right - 12) void getCurrentWindow().startDragging();
        }}
        onScroll={(e) => {
          const h = e.currentTarget;
          dangXemLai.current = h.scrollHeight - h.scrollTop - h.clientHeight > 24;
        }}
        style={{
          maxWidth: '100%',
          maxHeight: '100%',
          // Lúc chơi câu dài tràn thì cắt phía TRÊN (câu cũ nhất) — đã cuộn sẵn về cuối.
          overflowY: movable ? 'auto' : 'hidden',
          boxSizing: 'border-box',
          padding: '12px 22px 14px',
          borderRadius: 16,
          background: 'rgba(11,12,14,0.78)',
          backdropFilter: 'blur(12px)',
          border: movable ? '1px dashed #8a8f9a' : '1px solid rgba(255,255,255,0.08)',
          textAlign: 'center',
          cursor: movable ? 'move' : 'default',
          opacity: show ? 1 : 0,
          transition: 'opacity 0.3s',
        }}
      >
        {cues.length === 0 && (
          <div style={{ fontSize: 22, lineHeight: '30px', fontWeight: 600, color: '#ecebe6' }}>
            Kéo để dời phụ đề
          </div>
        )}
        {hien.map((c, i) => {
          const tuoi = Math.min(hien.length - 1 - i, MAU.length - 1); // 0 = mới nhất
          return (
            <div key={i} style={{ marginTop: i === 0 ? 0 : 8 }}>
              {c.src !== c.out && (
                <div style={{ fontSize: 12, lineHeight: '16px', color: '#8a8f9a' }}>{c.src}</div>
              )}
              <div
                style={{
                  fontSize: CO[tuoi],
                  lineHeight: 1.35,
                  fontWeight: tuoi === 0 ? 600 : 500,
                  color: MAU[tuoi],
                }}
              >
                {c.out}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
