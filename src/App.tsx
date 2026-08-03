import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Config, MODES, Voice } from './types';

const PURPLE = '#7c3aed';
const RED = '#b91c1c';
const GREEN = '#16a34a';

const pad = { padding: '0 18px' };

export default function App() {
  const [cfg, setCfg] = useState<Config | null>(null);
  const [voices, setVoices] = useState<Voice[]>([]);
  const [running, setRunning] = useState(false);
  const [status, setStatus] = useState('');
  // Lỗi nạp phải có state RIÊNG: dùng chung `status` thì event "status" của worker ghi đè mất.
  const [err, setErr] = useState('');
  const [log, setLog] = useState<string[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Lỗi command phải HIỆN LÊN — nuốt lặng thì UI kẹt ở "Đang tải…" không rõ vì sao.
    const fail = (e: unknown) => setErr(`Lỗi: ${String(e)}`);
    void invoke<Config>('get_config').then(setCfg, fail);
    void invoke<Voice[]>('list_voices').then(setVoices, fail);
    void invoke<boolean>('is_running').then(setRunning, fail);
    const un = [
      listen<string>('status', (e) => setStatus(e.payload)),
      // giữ 200 dòng gần nhất — log dài không giúp gì mà tốn RAM
      listen<string>('text', (e) => setLog((l) => [...l, e.payload].slice(-200))),
      listen<boolean>('running', (e) => setRunning(e.payload)), // hotkey Ctrl+Alt+S đổi trạng thái
      listen<Config>('config', (e) => setCfg(e.payload)), // overlay chọn xong vùng ở cửa sổ khác
    ];
    return () => {
      void Promise.all(un).then((fns) => fns.forEach((f) => f()));
    };
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo(0, logRef.current.scrollHeight);
  }, [log]);

  if (!cfg) return <div style={{ padding: 18 }}>{err || 'Đang tải…'}</div>;

  const ex = cfg.exclude_region;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh' }}>
      <div style={{ ...pad, paddingTop: 16 }}>
        <button
          style={{ width: '100%', height: 46, fontSize: 15, background: running ? RED : GREEN }}
          onClick={() => void invoke<boolean>('toggle').then(setRunning)}
        >
          {running ? '⏸  DỪNG ĐỌC' : '▶  BẮT ĐẦU ĐỌC'}
        </button>
      </div>

      <div style={{ ...pad, display: 'flex', gap: 8, marginTop: 8 }}>
        <button
          style={{ flex: 1, background: PURPLE }}
          onClick={() => void invoke('pick_region', { kind: 'scan' })}
        >
          SỬA VÙNG QUÉT
        </button>
        <button
          style={{ flex: 1, background: RED }}
          onClick={() => void invoke('pick_region', { kind: 'exclude' })}
        >
          VÙNG LOẠI TRỪ
        </button>
      </div>

      <div style={{ ...pad, fontSize: 12, color: '#9ca3af', marginTop: 6 }}>
        Vùng quét: {cfg.region.width}×{cfg.region.height} tại ({cfg.region.left},{cfg.region.top})
        {ex && (
          <>
            {' · loại trừ '}
            {ex.width}×{ex.height}{' '}
            <button
              style={{ background: '#374151', padding: '1px 6px', fontSize: 11 }}
              onClick={() => void invoke<Config>('clear_exclude').then(setCfg)}
            >
              xoá
            </button>
          </>
        )}
      </div>

      <div style={pad}>
        <label>CHẾ ĐỘ</label>
        <select
          value={cfg.mode}
          onChange={(e) => void invoke<Config>('set_mode', { mode: e.target.value }).then(setCfg)}
        >
          {MODES.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
        </select>

        <label>GIỌNG ĐỌC</label>
        <select
          value={cfg.voice_name}
          disabled={voices.length === 0}
          onChange={(e) => {
            setCfg({ ...cfg, voice_name: e.target.value });
            void invoke('set_voice', { name: e.target.value });
          }}
        >
          {voices.length === 0 ? (
            <option>(chưa cài giọng tiếng Việt)</option>
          ) : (
            voices.map((v) => (
              <option key={v.id} value={v.id}>
                {v.label}
              </option>
            ))
          )}
        </select>

        <label>ÂM LƯỢNG &nbsp; {cfg.volume}%</label>
        <input
          type="range"
          min={0}
          max={100}
          value={cfg.volume}
          onChange={(e) => {
            const volume = Number(e.target.value);
            setCfg({ ...cfg, volume });
            void invoke('set_volume', { volume });
          }}
        />

        <label>TỐC ĐỘ &nbsp; {cfg.speed.toFixed(1)}x</label>
        <input
          type="range"
          min={0.5}
          max={2}
          step={0.1}
          value={cfg.speed}
          onChange={(e) => {
            const speed = Number(e.target.value);
            setCfg({ ...cfg, speed });
            void invoke('set_speed', { speed });
          }}
        />
      </div>

      <div
        style={{ ...pad, color: err ? '#f87171' : '#9ca3af', fontSize: 12, margin: '14px 0 4px' }}
      >
        {err || status || 'Ctrl+Alt+S bật/tắt · Ctrl+Alt+R đọc lại'}
      </div>
      <div
        ref={logRef}
        style={{
          flex: 1,
          overflowY: 'auto',
          margin: '0 18px 16px',
          padding: 8,
          background: '#111',
          borderRadius: 6,
          whiteSpace: 'pre-wrap',
        }}
      >
        {log.join('\n')}
      </div>
    </div>
  );
}
