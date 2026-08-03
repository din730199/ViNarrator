import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Region } from '../types';

type Point = { x: number; y: number };

// Toạ độ CSS của webview -> toạ độ MÀN HÌNH vật lý (đúng thứ capture.rs/xcap dùng).
// Bắt buộc nhân scaleFactor: máy scale 125/150% sẽ lệch vùng nếu bỏ qua.
type Frame = { originX: number; originY: number; scale: number };

export default function Overlay() {
  const [kind, setKind] = useState('scan');
  const [start, setStart] = useState<Point | null>(null);
  const [cur, setCur] = useState<Point | null>(null);
  const frame = useRef<Frame>({ originX: 0, originY: 0, scale: 1 });

  useEffect(() => {
    const win = getCurrentWindow();
    const un = listen<string>('pick_start', async (e) => {
      setKind(e.payload);
      setStart(null);
      setCur(null);
      const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
      frame.current = { originX: pos.x, originY: pos.y, scale };
    });
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const finish = (region: Region | null) => {
    setStart(null);
    setCur(null);
    // Cửa sổ chính cập nhật qua event "config" do Rust emit — không phải giá trị trả về ở đây.
    void invoke('region_picked', { kind, region });
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') finish(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const toScreen = (p: Point): Point => {
    const { originX, originY, scale } = frame.current;
    return { x: Math.round(originX + p.x * scale), y: Math.round(originY + p.y * scale) };
  };

  const onUp = (e: React.MouseEvent) => {
    if (!start) return;
    const a = toScreen(start);
    const b = toScreen({ x: e.clientX, y: e.clientY });
    const region: Region = {
      left: Math.min(a.x, b.x),
      top: Math.min(a.y, b.y),
      width: Math.abs(b.x - a.x),
      height: Math.abs(b.y - a.y),
    };
    // Kéo hụt (click nhầm) -> coi như huỷ, giống ngưỡng 5px của bản tkinter.
    finish(region.width > 5 && region.height > 5 ? region : null);
  };

  const box =
    start && cur
      ? {
          left: Math.min(start.x, cur.x),
          top: Math.min(start.y, cur.y),
          width: Math.abs(cur.x - start.x),
          height: Math.abs(cur.y - start.y),
        }
      : null;

  return (
    <div
      style={{ position: 'fixed', inset: 0, background: 'rgba(17,17,17,0.25)' }}
      onMouseDown={(e) => {
        setStart({ x: e.clientX, y: e.clientY });
        setCur({ x: e.clientX, y: e.clientY });
      }}
      onMouseMove={(e) => start && setCur({ x: e.clientX, y: e.clientY })}
      onMouseUp={onUp}
    >
      <div
        style={{
          position: 'absolute',
          top: '4%',
          width: '100%',
          textAlign: 'center',
          color: '#fff',
          textShadow: '0 1px 4px #000',
          fontSize: 16,
        }}
      >
        {kind === 'exclude'
          ? 'Kéo chọn VÙNG LOẠI TRỪ (bỏ qua)  •  Esc huỷ'
          : 'Kéo chọn VÙNG QUÉT (phụ đề/thoại)  •  Esc huỷ'}
      </div>
      {box && (
        <div
          style={{
            position: 'absolute',
            ...box,
            border: `2px solid ${kind === 'exclude' ? '#b91c1c' : '#ef4444'}`,
            background: 'rgba(255,255,255,0.08)',
          }}
        />
      )}
    </div>
  );
}
