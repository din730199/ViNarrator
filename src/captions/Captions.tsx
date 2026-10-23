import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Config, Spoken } from '../types';

// Hết thoại bao lâu thì ẩn hộp đi: cửa sổ vẫn phủ vùng đó trên game, ẩn cho đỡ vướng mắt.
const IDLE_MS = 8000;

export default function Captions() {
  const [cue, setCue] = useState<Spoken | null>(null);
  const [visible, setVisible] = useState(false);
  const [movable, setMovable] = useState(false);

  useEffect(() => {
    void invoke<Config>('get_config').then((c) => setMovable(c.captions_movable));
    const un = [
      listen<Spoken>('text', (e) => {
        setCue(e.payload);
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
  }, [cue, visible]);

  const show = visible || movable;
  return (
    <div
      style={{ height: '100%', display: 'flex', alignItems: 'flex-end', justifyContent: 'center' }}
    >
      <div
        onMouseDown={() => movable && void getCurrentWindow().startDragging()}
        style={{
          maxWidth: '100%',
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
        {cue && cue.src !== cue.out && (
          <div style={{ fontSize: 13, lineHeight: '18px', color: '#8a8f9a', marginBottom: 2 }}>
            {cue.src}
          </div>
        )}
        <div style={{ fontSize: 22, lineHeight: '30px', fontWeight: 600, color: '#ecebe6' }}>
          {cue ? cue.out : 'Kéo để dời phụ đề'}
        </div>
      </div>
    </div>
  );
}
